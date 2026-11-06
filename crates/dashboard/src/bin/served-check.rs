//! Proves a rebuilt preparation serves what the one it replaces served: asks
//! every product a family serves a fixed, broad set of requests of a
//! reference store and keeps the replies, then asks the same requests of a
//! candidate store and compares the replies
//! ([`dashboard::prepared::served_check`]).
//! `served-check record --family F --recording R --store S --work W --out T [--columnar C]`
//! `served-check check --recording R --store S --work W --reference T|DIR --report P|DIR [--families F,...] [--threads N] [--ignore PATH]...`
//! A folder of transcripts is checked in one process, every request of them `--threads` at a time.
//! Both also take `[--game G --user U]` (else BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR)
//! and `[--allow-unverified]`. A store is its folder,
//! `…\HOI4Replay\prepared\<store>`; `--work` keeps the content cache.
//! Exits 1 on a difference, failed family or unchecked product, 2 on a setup error.

use dashboard::{config::ContentOptions, prepared::served_check};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufReader, BufWriter},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

#[path = "served-check/process.rs"]
mod process;

#[path = "support/shader_compiler.rs"]
mod shader_compiler;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str =
    "served-check record --families all|F1,F2,... --recording R --store S --work W --out-dir D [--columnar C] [--threads N]
served-check record --family F --recording R --store S --work W --out T [--columnar C] [--threads N]
served-check check --recording R --store S --work W --reference T|DIR --report P|DIR [--families F1,F2,...] [--threads N] [--scopes P] [--ignore PATH]...
each also [--game G --user U] [--allow-unverified]";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

#[derive(Default)]
struct Options {
    family: Option<String>,
    families: Option<String>,
    recording: Option<PathBuf>,
    store: Option<PathBuf>,
    work: Option<PathBuf>,
    out: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    threads: Option<NonZeroUsize>,
    columnar: Option<PathBuf>,
    reference: Option<PathBuf>,
    report: Option<PathBuf>,
    /// Where `check` writes the CPU its queries charged, by label.
    scopes: Option<PathBuf>,
    game: Option<PathBuf>,
    user: Option<PathBuf>,
    /// Dotted reply paths not compared, `*` for any member or element.
    ignore: Vec<Vec<String>>,
    allow_unverified: bool,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<Self> {
        let mut options = Self::default();
        while let Some(flag) = args.next() {
            if flag == "--allow-unverified" {
                options.allow_unverified = true;
                continue;
            }
            let value = args.next().ok_or_else(|| invalid(USAGE))?;
            let path = Some(PathBuf::from(&value));
            match flag.to_str() {
                Some("--family") => options.family = Some(value.to_string_lossy().into_owned()),
                Some("--families") => options.families = Some(value.to_string_lossy().into_owned()),
                Some("--recording") => options.recording = path,
                Some("--store") => options.store = path,
                Some("--work") => options.work = path,
                Some("--out") => options.out = path,
                Some("--out-dir") => options.out_dir = path,
                Some("--threads") => {
                    options.threads = Some(
                        value
                            .to_str()
                            .and_then(|value| value.parse().ok())
                            .ok_or_else(|| invalid("--threads must be a positive integer"))?,
                    );
                }
                Some("--columnar") => options.columnar = path,
                Some("--reference") => options.reference = path,
                Some("--report") => options.report = path,
                Some("--scopes") => options.scopes = path,
                Some("--game") => options.game = path,
                Some("--user") => options.user = path,
                Some("--ignore") => options.ignore.push(
                    value
                        .to_string_lossy()
                        .split('.')
                        .map(str::to_owned)
                        .collect(),
                ),
                _ => return Err(invalid(USAGE)),
            }
        }
        Ok(options)
    }

    /// The content the stores were prepared under, from the flags or else the
    /// environment; none without either.
    fn content(&self, work: &Path) -> io::Result<Option<ContentOptions>> {
        let folder = |flag: &Option<PathBuf>, variable: &str| {
            flag.clone()
                .or_else(|| std::env::var_os(variable).map(PathBuf::from))
        };
        let (game_dir, user_dir) = match (
            folder(&self.game, "BATTLE_PLAN_GAME_DIR"),
            folder(&self.user, "BATTLE_PLAN_USER_DIR"),
        ) {
            (Some(game), Some(user)) => (game, user),
            (None, None) => return Ok(None),
            _ => return Err(invalid("Give both the game and user folders, or neither")),
        };
        let cache_dir = work.join("content");
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Some(ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir,
                user_dir,
                cache_dir,
            },
            allow_unverified: self.allow_unverified,
            presentation: Default::default(),
        }))
    }
}

fn required<'a, T: ?Sized>(value: Option<&'a T>) -> io::Result<&'a T> {
    value.ok_or_else(|| invalid(USAGE))
}

struct Job {
    family: String,
    owners: Vec<served_check::Owner>,
    transcript: PathBuf,
    report: Option<PathBuf>,
    bytes: u64,
}

#[derive(Serialize)]
struct Recorded {
    #[serde(rename = "shaderCompiler")]
    shader_compiler: &'static str,
    family: String,
    seconds: f64,
    branches: usize,
    owners: Vec<served_check::Recorded>,
    unchecked: Vec<String>,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Recorded {
    fn success(&self) -> bool {
        self.error.is_none() && self.unchecked.is_empty()
    }
}

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temporary(path: &Path) -> io::Result<(Temporary, File)> {
    let name = path
        .file_name()
        .ok_or_else(|| invalid("Output must name a file"))?;
    let mut name = name.to_os_string();
    name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let path = path.with_file_name(name);
    let file = File::create_new(&path)?;
    Ok((Temporary(path), file))
}

fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let (temporary, file) = temporary(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    drop(
        writer
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?,
    );
    std::fs::rename(&temporary.0, path)
}

fn absent(path: &Path) -> io::Result<()> {
    if path.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Output already exists: {}", path.display()),
        ));
    }
    Ok(())
}

fn jobs(options: &Options) -> io::Result<Vec<Job>> {
    let (families, directory) = match (
        &options.family,
        &options.families,
        &options.out,
        &options.out_dir,
    ) {
        (Some(family), None, Some(_), None) => (served_check::families(family)?, None),
        (None, Some(families), None, Some(directory)) => {
            (served_check::families(families)?, Some(directory))
        }
        _ => return Err(invalid(USAGE)),
    };
    if directory.is_none() && families.len() != 1 {
        return Err(invalid("--family selects exactly one family"));
    }
    if let Some(directory) = directory {
        std::fs::create_dir_all(directory)?;
        absent(&directory.join("summary.json"))?;
    }
    let mut jobs = Vec::with_capacity(families.len());
    for family in families {
        let transcript = directory.map_or_else(
            || options.out.clone().unwrap(),
            |directory| directory.join(format!("{family}.zst")),
        );
        let report = directory.map(|directory| directory.join(format!("{family}.json")));
        absent(&transcript)?;
        if let Some(report) = &report {
            absent(report)?;
        }
        jobs.push(Job {
            owners: served_check::owners(&family)?,
            family,
            transcript,
            report,
            bytes: 0,
        });
    }
    Ok(jobs)
}

fn record_family(
    job: &Job,
    served: &served_check::Served,
    branches: &[dashboard_base::prepared::served::Branch],
    shader_compiler: &'static str,
) -> Recorded {
    let _returned = dashboard::allocator::ReturnFreed;
    let started = Instant::now();
    let mut summary = Recorded {
        shader_compiler,
        family: job.family.clone(),
        seconds: 0.0,
        branches: branches.len(),
        owners: Vec::new(),
        unchecked: Vec::new(),
        status: "failed",
        error: None,
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<()> {
        if job.owners.is_empty() {
            return Err(invalid(format!("{} serves no prepared query", job.family)));
        }
        served.require_family(&job.family)?;
        let (temporary, file) = temporary(&job.transcript)?;
        let mut transcript = BufWriter::new(zstd::stream::write::Encoder::new(file, 3)?);
        summary.owners = served_check::record(
            &job.family,
            served.revision(),
            &job.owners,
            branches,
            Some(served.recording()),
            &mut |product, params| served.serve(product, params),
            &mut transcript,
        )?;
        drop(
            transcript
                .into_inner()
                .map_err(io::IntoInnerError::into_error)?
                .finish()?,
        );
        std::fs::rename(&temporary.0, &job.transcript)?;
        Ok(())
    }));
    summary.error = match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error.to_string()),
        Err(panic) => Some(format!(
            "The family panicked: {}",
            panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic")
        )),
    };
    summary.unchecked = summary
        .owners
        .iter()
        .filter(|owner| owner.unchecked())
        .map(|owner| owner.owner.clone())
        .collect();
    if summary.error.is_none() {
        summary.status = if summary.owners.iter().any(|owner| owner.refused > 0) {
            "refused"
        } else if !summary.unchecked.is_empty() {
            "unchecked"
        } else {
            "recorded"
        };
    }
    summary.seconds = started.elapsed().as_secs_f64();
    if let Some(path) = &job.report
        && let Err(error) = write_json(path, &summary)
    {
        summary.status = "failed";
        let previous = summary
            .error
            .take()
            .map(|error| format!("{error}; "))
            .unwrap_or_default();
        summary.error = Some(format!(
            "{previous}Could not write {}: {error}",
            path.display()
        ));
    }
    summary
}

fn record(
    options: &Options,
    recording: &Path,
    folder: &Path,
    content: Option<ContentOptions>,
    started: Instant,
    shader_compiler: &'static str,
) -> io::Result<bool> {
    let mut jobs = jobs(options)?;
    let segments = options
        .columnar
        .as_deref()
        .map(served_check::segments)
        .transpose()?
        .unwrap_or_default();
    let served = served_check::Served::open_shared(recording, folder, content)?;
    let branches = served.branches(&segments);
    let bytes = served.family_bytes()?;
    for job in &mut jobs {
        let name = served_check::family_name(&job.family)?;
        job.bytes = bytes.get(name).copied().unwrap_or(0);
    }
    jobs.sort_by_key(|job| std::cmp::Reverse(job.bytes));
    let threads = options
        .threads
        .map(Ok)
        .unwrap_or_else(std::thread::available_parallelism)?;
    let recorded = served_check::parallel(&jobs, threads, |job| {
        record_family(job, &served, &branches, shader_compiler)
    })?;
    for family in &recorded {
        println!("{}", serde_json::to_string_pretty(family)?);
    }
    let metrics = process::metrics();
    let summary = json!({
        "shaderCompiler": shader_compiler,
        "families": recorded, "wall_seconds": started.elapsed().as_secs_f64(),
        "cpu_seconds": metrics.cpu_seconds, "peak_commit_bytes": metrics.peak_commit_bytes,
        "threads": threads.get().min(jobs.len()),
    });
    if let Some(directory) = &options.out_dir {
        write_json(&directory.join("summary.json"), &summary)?;
    }
    eprintln!(
        "served-check process: {}",
        json!({
            "wall_seconds": summary["wall_seconds"], "cpu_seconds": metrics.cpu_seconds,
            "peak_commit_bytes": metrics.peak_commit_bytes,
        })
    );
    Ok(recorded.iter().all(Recorded::success))
}

fn check(
    options: &Options,
    recording: &Path,
    folder: &Path,
    content: Option<ContentOptions>,
    shader_compiler: &str,
) -> io::Result<bool> {
    let reference = required(options.reference.as_deref())?;
    let report = required(options.report.as_deref())?;
    // A folder of transcripts is checked whole, each family's report in the report folder.
    let checked: Vec<(PathBuf, PathBuf)> = if reference.is_dir() {
        let selected = options
            .families
            .as_deref()
            .map(served_check::families)
            .transpose()?;
        std::fs::create_dir_all(report)?;
        let mut transcripts = Vec::new();
        for entry in std::fs::read_dir(reference)? {
            let path = entry?.path();
            if path.extension().is_some_and(|extension| extension == "zst")
                && let Some(family) = path.file_stem().and_then(|stem| stem.to_str())
                && selected
                    .as_ref()
                    .is_none_or(|selected| selected.iter().any(|name| name == family))
            {
                let bytes = std::fs::metadata(&path)?.len();
                transcripts.push((bytes, path.clone(), report.join(format!("{family}.json"))));
            }
        }
        if let Some(selected) = &selected {
            let missing: Vec<&str> = selected
                .iter()
                .filter(|family| {
                    !transcripts
                        .iter()
                        .any(|(_, path, _): &(u64, PathBuf, PathBuf)| {
                            path.file_stem().is_some_and(|stem| stem == family.as_str())
                        })
                })
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                return Err(invalid(format!("No transcript of {}", missing.join(", "))));
            }
        }
        if transcripts.is_empty() {
            return Err(invalid(format!(
                "{} holds no transcript",
                reference.display()
            )));
        }
        // The largest first, so the last requests asked are short ones.
        transcripts.sort_by_key(|(bytes, ..)| std::cmp::Reverse(*bytes));
        transcripts
            .into_iter()
            .map(|(_, transcript, report)| (transcript, report))
            .collect()
    } else {
        vec![(reference.to_owned(), report.to_owned())]
    };
    let served = served_check::Served::open_shared(recording, folder, content)?;
    let shape = |branch: &dashboard_base::prepared::served::Branch| {
        (
            branch.id,
            branch.first_hour,
            branch.last_hour,
            branch.windows.clone(),
            branch.gaps.clone(),
        )
    };
    let branches = served.branches(&BTreeMap::new());
    let mut families = Vec::new();
    let mut readers: Vec<Box<dyn io::BufRead + Send>> = Vec::new();
    let mut success = true;
    for (path, report) in checked {
        let mut transcript = BufReader::new(zstd::stream::read::Decoder::new(File::open(&path)?)?);
        let header = served_check::read_header(&mut transcript)?;
        let refused = if header.revision != served.revision() {
            Some("The transcript was recorded of another revision of the recording".to_owned())
        } else if !header
            .branches
            .iter()
            .map(shape)
            .eq(branches.iter().map(shape))
        {
            Some("The transcript's branches are not the recording's".to_owned())
        } else {
            served
                .require_family(&header.family)
                .err()
                .map(|error| error.to_string())
        };
        if let Some(refused) = refused {
            println!("{}: not checked: {refused}", header.family);
            success = false;
            continue;
        }
        families.push((header.family, report, path));
        readers.push(Box::new(transcript));
    }
    let threads = options
        .threads
        .map(Ok)
        .unwrap_or_else(std::thread::available_parallelism)?;
    if options.scopes.is_some() {
        replay_frame::cpu::enable();
    }
    let started = Instant::now();
    // One transcript never repeats a request: only several need counting.
    let shared = if families.len() > 1 {
        served_check::shared_requests(families.iter().map(
            |(_, _, path)| -> io::Result<Box<dyn io::BufRead>> {
                let mut transcript =
                    BufReader::new(zstd::stream::read::Decoder::new(File::open(path)?)?);
                served_check::read_header(&mut transcript)?;
                Ok(Box::new(transcript))
            },
        ))?
    } else {
        Default::default()
    };
    let reports = served_check::check_parallel(
        readers,
        &|product: &str, params: &Value| served.serve(product, params),
        shared,
        &options.ignore,
        threads,
    )?;
    let mut differing = 0;
    for ((family, path, _), (report, seconds)) in families.iter().zip(&reports) {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "shaderCompiler": shader_compiler,
                "family": family,
                "seconds": seconds,
                "report": report,
            }))?,
        )?;
        for (product, tally) in &report.products {
            if tally.differing > 0 {
                println!(
                    "{family} {product}: {} asked, {} answered, {} pending, {} refused, {} differing",
                    tally.queries, tally.answered, tally.pending, tally.refused, tally.differing
                );
            }
            if let Some(first) = &tally.first {
                println!(
                    "  first at `{}` of {} {}",
                    first.difference.path, first.product, first.params
                );
            }
        }
        println!(
            "{family}: {} of {} replies differ, served in {seconds:.1} s",
            report.differing, report.queries
        );
        differing += report.differing;
    }
    if let Some(path) = &options.scopes {
        write_json(path, &replay_frame::cpu::snapshot())?;
    }
    println!(
        "{} families, {differing} differing replies, {:.1} s on {threads} threads",
        families.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(success && differing == 0)
}

fn run() -> io::Result<bool> {
    let started = Instant::now();
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().ok_or_else(|| invalid(USAGE))?;
    let options = Options::parse(args)?;
    let recording = required(options.recording.as_deref())?;
    let folder = required(options.store.as_deref())?;
    let work = required(options.work.as_deref())?;
    let shader_compiler = shader_compiler::configure();
    // Set before any other thread starts: the store is read under it.
    unsafe { std::env::set_var("LOCALAPPDATA", served_check::store_root(folder)?) };
    std::fs::create_dir_all(work)?;
    let content = options.content(work)?;
    match mode.to_str() {
        Some("record") => record(
            &options,
            recording,
            folder,
            content,
            started,
            shader_compiler,
        ),
        Some("check") => check(&options, recording, folder, content, shader_compiler),
        _ => Err(invalid(USAGE)),
    }
}

fn main() -> ExitCode {
    dashboard::allocator::return_freed_memory_promptly();
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("served-check: {error}");
            ExitCode::from(2)
        }
    }
}
