//! Proves a rebuilt preparation serves what the one it replaces served: asks
//! every product a family serves a fixed, broad set of requests of a
//! reference store and keeps the replies, then asks the same requests of a
//! candidate store and compares the replies
//! ([`dashboard::prepared::served_check`]).
//! `served-check record --family F --recording R --store S --work W --out T [--columnar C]`
//! `served-check check --recording R --store S --work W --reference T --report P`
//! Both also take `[--game G --user U]` (else BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR)
//! and `[--allow-unverified]`. A store is its folder,
//! `…\HOI4Replay\prepared\<store>`; `--work` keeps the content cache.
//! Exits 1 when a reply differs or a product checks nothing, 2 on an error.

use dashboard::{config::ContentOptions, prepared::served_check};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufReader, BufWriter},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str =
    "served-check record --family F --recording R --store S --work W --out T [--columnar C]
served-check check --recording R --store S --work W --reference T --report P
each also [--game G --user U] [--allow-unverified]";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

#[derive(Default)]
struct Options {
    family: Option<String>,
    recording: Option<PathBuf>,
    store: Option<PathBuf>,
    work: Option<PathBuf>,
    out: Option<PathBuf>,
    columnar: Option<PathBuf>,
    reference: Option<PathBuf>,
    report: Option<PathBuf>,
    game: Option<PathBuf>,
    user: Option<PathBuf>,
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
                Some("--recording") => options.recording = path,
                Some("--store") => options.store = path,
                Some("--work") => options.work = path,
                Some("--out") => options.out = path,
                Some("--columnar") => options.columnar = path,
                Some("--reference") => options.reference = path,
                Some("--report") => options.report = path,
                Some("--game") => options.game = path,
                Some("--user") => options.user = path,
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
        engine_executable::executable_tables::initialize(&game_dir)?;
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

fn record(
    options: &Options,
    recording: &Path,
    folder: &Path,
    content: Option<ContentOptions>,
) -> io::Result<bool> {
    let family = required(options.family.as_deref())?;
    let out = required(options.out.as_deref())?;
    let owners = served_check::owners(family)?;
    if owners.is_empty() {
        return Err(invalid(format!("{family} serves no prepared query")));
    }
    let segments = options
        .columnar
        .as_deref()
        .map(served_check::segments)
        .transpose()?
        .unwrap_or_default();
    let served = served_check::Served::open(recording, folder, content, family)?;
    let branches = served.branches(&segments);
    let started = Instant::now();
    let mut transcript = BufWriter::new(zstd::stream::write::Encoder::new(
        File::create_new(out)?,
        3,
    )?);
    let recorded = served_check::record(
        family,
        served.revision(),
        &owners,
        &branches,
        &mut |product: &str, params: &Value| served.serve(product, params),
        &mut transcript,
    )?;
    transcript
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?
        .finish()?;
    let unchecked: Vec<&str> = recorded
        .iter()
        .filter(|owner| owner.unchecked())
        .map(|owner| owner.owner.as_str())
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "family": family,
            "seconds": started.elapsed().as_secs_f64(),
            "branches": branches.len(),
            "owners": &recorded,
            "unchecked": &unchecked,
        }))?
    );
    Ok(unchecked.is_empty())
}

fn check(
    options: &Options,
    recording: &Path,
    folder: &Path,
    content: Option<ContentOptions>,
) -> io::Result<bool> {
    let reference = required(options.reference.as_deref())?;
    let path = required(options.report.as_deref())?;
    let mut transcript = BufReader::new(zstd::stream::read::Decoder::new(File::open(reference)?)?);
    let header = served_check::read_header(&mut transcript)?;
    let served = served_check::Served::open(recording, folder, content, &header.family)?;
    if header.revision != served.revision() {
        return Err(invalid(
            "The transcript was recorded of another revision of the recording",
        ));
    }
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
    if !header
        .branches
        .iter()
        .map(shape)
        .eq(branches.iter().map(shape))
    {
        return Err(invalid("The transcript's branches are not the recording's"));
    }
    let started = Instant::now();
    let report = served_check::check(&mut transcript, &mut |product: &str, params: &Value| {
        served.serve(product, params)
    })?;
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "family": &header.family,
            "seconds": started.elapsed().as_secs_f64(),
            "report": &report,
        }))?,
    )?;
    for (product, tally) in &report.products {
        println!(
            "{product}: {} asked, {} answered, {} pending, {} refused, {} differing",
            tally.queries, tally.answered, tally.pending, tally.refused, tally.differing
        );
        if let Some(first) = &tally.first {
            println!(
                "  first at `{}` of {} {}",
                first.difference.path, first.product, first.params
            );
        }
    }
    println!(
        "{}: {} of {} replies differ; the report is {}",
        header.family,
        report.differing,
        report.queries,
        path.display()
    );
    Ok(report.differing == 0)
}

fn run() -> io::Result<bool> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().ok_or_else(|| invalid(USAGE))?;
    let options = Options::parse(args)?;
    let recording = required(options.recording.as_deref())?;
    let folder = required(options.store.as_deref())?;
    let work = required(options.work.as_deref())?;
    // Set before any other thread starts: the store is read under it.
    unsafe { std::env::set_var("LOCALAPPDATA", served_check::store_root(folder)?) };
    std::fs::create_dir_all(work)?;
    let content = options.content(work)?;
    match mode.to_str() {
        Some("record") => record(&options, recording, folder, content),
        Some("check") => check(&options, recording, folder, content),
        _ => Err(invalid(USAGE)),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("served-check: {error}");
            ExitCode::from(2)
        }
    }
}
