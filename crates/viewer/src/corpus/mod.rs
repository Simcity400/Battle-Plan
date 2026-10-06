//! `viewer --mod-corpus`: every content reader over vanilla and over each
//! installed mod on its own, with every diagnostic the readers give, so a
//! change's effect on real content can be read off two runs
//! (`viewer --mod-corpus-diff`). See the viewer README.

mod readers;
mod report;
mod stacks;
pub(crate) mod values;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

pub const USAGE: &str =
    "viewer --mod-corpus --out <directory> [--game-dir <directory>] [--user-dir <directory>]
         [--workshop <directory>] [--cache <directory>] [--only <stack,...>] [--jobs <n>] [--values[=text]]
viewer --mod-corpus-diff <base run> <head run> [--out <file.md>] [--examples <n>]";

/// The stack with no mod, which every other stack mounts beneath its mod.
const VANILLA: &str = "vanilla";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Runs `--mod-corpus` or `--mod-corpus-diff` if `args` start with one.
pub fn dispatch(args: &[OsString]) -> Option<io::Result<()>> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first()?.to_str()? {
        "--mod-corpus" => Some(run(rest)),
        "--mod-corpus-diff" => Some(run_diff(rest)),
        _ => None,
    }
}

struct Options {
    game_dir: PathBuf,
    user_dir: PathBuf,
    workshop: Option<PathBuf>,
    out: PathBuf,
    cache: Option<PathBuf>,
    only: Option<Vec<String>>,
    jobs: usize,
    values: Option<values::Mode>,
}

fn value(args: &mut std::slice::Iter<'_, OsString>, name: &str) -> io::Result<OsString> {
    args.next()
        .cloned()
        .ok_or_else(|| invalid(format!("{name} needs a value\n{USAGE}")))
}

fn number(text: OsString, name: &str) -> io::Result<usize> {
    text.to_str()
        .and_then(|text| text.parse().ok())
        .filter(|&count| count > 0)
        .ok_or_else(|| invalid(format!("{name} needs a positive count")))
}

fn parse(args: &[OsString]) -> io::Result<Options> {
    let (mut game_dir, mut user_dir, mut workshop, mut out, mut cache, mut only) =
        (None, None, None, None, None, None);
    let mut jobs = 4;
    let mut values = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--game-dir") => game_dir = Some(PathBuf::from(value(&mut args, "--game-dir")?)),
            Some("--user-dir") => user_dir = Some(PathBuf::from(value(&mut args, "--user-dir")?)),
            Some("--workshop") => workshop = Some(PathBuf::from(value(&mut args, "--workshop")?)),
            Some("--out") => out = Some(PathBuf::from(value(&mut args, "--out")?)),
            Some("--cache") => cache = Some(PathBuf::from(value(&mut args, "--cache")?)),
            Some("--only") => {
                only = Some(
                    value(&mut args, "--only")?
                        .to_string_lossy()
                        .split(',')
                        .map(str::to_owned)
                        .collect(),
                )
            }
            Some("--jobs") => jobs = number(value(&mut args, "--jobs")?, "--jobs")?,
            Some("--values") => values = Some(values::Mode::Digest),
            Some("--values=text") => values = Some(values::Mode::Text),
            _ => {
                return Err(invalid(format!(
                    "unknown argument {}\n{USAGE}",
                    arg.to_string_lossy()
                )));
            }
        }
    }
    // The folders the viewer is configured with, unless named.
    let config = dashboard::Config::load().ok();
    let configured = |path: Option<PathBuf>, pick: fn(&dashboard::Config) -> &PathBuf| {
        path.or_else(|| config.as_ref().map(|config| pick(config).clone()))
            .filter(|path| !path.as_os_str().is_empty())
    };
    Ok(Options {
        game_dir: configured(game_dir, |config| &config.game_dir)
            .ok_or_else(|| invalid(format!("no game folder configured or named\n{USAGE}")))?,
        user_dir: configured(user_dir, |config| &config.user_dir)
            .ok_or_else(|| invalid(format!("no user folder configured or named\n{USAGE}")))?,
        workshop,
        out: out.ok_or_else(|| invalid(format!("--out is required\n{USAGE}")))?,
        cache,
        only,
        jobs,
        values,
    })
}

fn run(args: &[OsString]) -> io::Result<()> {
    let options = parse(args)?;
    let started = Instant::now();
    let language = stacks::language(&options.user_dir)?;
    // The reading client's texture decode state, which font pages load under.
    let decode = installed_content::settings::read_keys(&options.user_dir)?.texture_decode();
    let (candidates, refused) = stacks::candidates(&options.user_dir, options.workshop.as_deref())?;
    for unusable in &refused {
        eprintln!(
            "mod-corpus: {} is not a usable mod descriptor: {}",
            unusable.path, unusable.why
        );
    }
    let mut plans = vec![stacks::Plan {
        id: VANILLA.into(),
        name: "Vanilla and DLC".into(),
        mods: Vec::new(),
        skipped: Vec::new(),
    }];
    plans.extend(stacks::plans(&candidates));
    if let Some(only) = &options.only {
        plans.retain(|plan| only.contains(&plan.id));
    }
    fs::create_dir_all(&options.out)?;
    let value_dir = options.out.join("values");
    // Do not let a reused run directory advertise stale values.
    if options.values.is_some() {
        if value_dir.exists() {
            fs::remove_dir_all(&value_dir)?;
        }
        fs::create_dir_all(&value_dir)?;
    }
    let scratch = options.out.join("scratch");
    let cache = options
        .cache
        .clone()
        .unwrap_or_else(|| scratch.join("hashes"));
    let dump_error = options.values.map(|_| Arc::new(Mutex::new(None)));
    let queue = Mutex::new(plans.iter());
    let reports = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..options.jobs.min(plans.len()) {
            scope.spawn(|| {
                loop {
                    let Some(plan) = queue.lock().unwrap_or_else(|e| e.into_inner()).next() else {
                        break;
                    };
                    let report = stack(
                        &options,
                        (&language, decode),
                        &candidates,
                        plan,
                        &scratch,
                        &cache,
                        dump_error.as_ref(),
                    );
                    eprintln!(
                        "mod-corpus: {} gathered in {} ms, read in {} ms: {} diagnostics{}",
                        report.stack,
                        report.gather_milliseconds,
                        report.read_milliseconds,
                        report.diagnostics.len(),
                        report
                            .gather_error
                            .as_ref()
                            .map(|error| format!(" (not gathered: {error})"))
                            .unwrap_or_default()
                    );
                    reports
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(report.stack.clone(), report);
                }
            });
        }
    });
    let reports = reports.into_inner().unwrap_or_else(|e| e.into_inner());
    if let Some(error) = dump_error
        && let Some(error) = error.lock().unwrap().take()
    {
        return Err(error);
    }
    report::write_run(&options.out, &reports)?;
    let mut summary = format!(
        "# Mod corpus\n\n{} stacks in {} ms with {} jobs.\n\n",
        reports.len(),
        started.elapsed().as_millis(),
        options.jobs
    );
    for unusable in &refused {
        summary.push_str(&format!(
            "- Not a usable mod descriptor: {}: {}\n",
            unusable.path, unusable.why
        ));
    }
    summary.push('\n');
    summary.push_str(&report::summary(&reports, 10));
    fs::write(options.out.join("summary.md"), &summary)?;
    eprintln!(
        "mod-corpus: {} stacks in {} ms; summary in {}",
        reports.len(),
        started.elapsed().as_millis(),
        options.out.join("summary.md").display()
    );
    Ok(())
}

/// Gathers one stack and runs every reader over it.
fn stack(
    options: &Options,
    (language, decode): (&str, installed_content::image::native::Decode),
    candidates: &[stacks::Candidate],
    plan: &stacks::Plan,
    scratch: &Path,
    cache: &Path,
    dump_error: Option<&values::Error>,
) -> report::StackReport {
    let mut report = report::StackReport {
        stack: plan.id.clone(),
        name: plan.name.clone(),
        skipped_dependencies: plan.skipped.clone(),
        ..Default::default()
    };
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.id == plan.id) {
        report.descriptor_file = Some(candidate.file.clone());
        // The rewritten descriptor has already been checked to parse back identically.
        if let Ok(descriptor) = installed_content::parser::parse(candidate.descriptor.as_bytes()) {
            let field = |key: &[u8]| {
                descriptor
                    .get(key)
                    .and_then(|value| value.scalar_str())
                    .unwrap_or_default()
                    .to_owned()
            };
            report.version = field(b"version");
            report.supported_version = field(b"supported_version");
        }
    }
    let started = Instant::now();
    // Each stack keeps its own hash cache: gather rewrites the whole file.
    let gathered = stacks::user_folder(&scratch.join(&plan.id), language, candidates, &plan.mods)
        .and_then(|user_dir| {
            installed_content::gather(&installed_content::GatherOptions {
                game_dir: options.game_dir.clone(),
                user_dir,
                cache_dir: cache.join(&plan.id),
                dlc_order: None,
            })
        });
    report.gather_milliseconds = started.elapsed().as_millis();
    let content = match gathered {
        Ok(content) => content,
        Err(error) => {
            report.gather_error = Some(error.to_string());
            return report;
        }
    };
    report.dlc = content.dlc.len();
    report.order_note = content.order_note.clone();
    report.mods = content
        .mods
        .iter()
        .map(|layer| report::Mounted {
            id: layer.id.clone(),
            name: layer.display_name.clone(),
            version: layer.declared_version.clone(),
        })
        .collect();
    let started = Instant::now();
    let values = dump_error.map(|error| {
        values::Writer::new(
            options.out.join("values").join(&plan.id),
            Arc::clone(error),
            options.values.unwrap(),
        )
    });
    let (runs, diagnostics, state_history) = readers::run(content, decode, values.clone());
    if let Some(values) = values {
        values.finish();
    }
    report.read_milliseconds = started.elapsed().as_millis();
    report.readers = runs;
    report.diagnostics = diagnostics;
    report.state_history = state_history.notes;
    report.state_history_states = state_history.states;
    report
}

fn run_diff(args: &[OsString]) -> io::Result<()> {
    let (mut runs, mut out, mut examples) = (Vec::new(), None, 20);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--out") => out = Some(PathBuf::from(value(&mut args, "--out")?)),
            Some("--examples") => examples = number(value(&mut args, "--examples")?, "--examples")?,
            _ => runs.push(PathBuf::from(arg)),
        }
    }
    let [base, head] = runs.as_slice() else {
        return Err(invalid(format!("two run folders are required\n{USAGE}")));
    };
    let mut text = report::diff(&report::read_run(base)?, &report::read_run(head)?, examples);
    text.push_str(&values::diff(base, head)?);
    match out {
        Some(path) => fs::write(path, text),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}
