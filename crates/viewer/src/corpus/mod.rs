
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
         [--workshop <directory>] [--cache <directory>] [--only <stack,...>] [--jobs <n>] [--values[=text]] [--sounds-only | --maps-only]
viewer --mod-corpus-diff <base run> <head run> [--out <file.md>] [--examples <n>]";

const VANILLA: &str = "vanilla";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

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
    sounds_only: bool,
    maps_only: bool,
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
    let mut sounds_only = false;
    let mut maps_only = false;
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
            Some("--sounds-only") => sounds_only = true,
            Some("--maps-only") => maps_only = true,
            _ => {
                return Err(invalid(format!(
                    "unknown argument {}\n{USAGE}",
                    arg.to_string_lossy()
                )));
            }
        }
    }
    if sounds_only && maps_only {
        return Err(invalid("give one of --sounds-only and --maps-only"));
    }
    let config = dashboard_core::config::Config::load().ok();
    let configured = |path: Option<PathBuf>, pick: fn(&dashboard_core::config::Config) -> &PathBuf| {
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
        sounds_only,
        maps_only,
    })
}

fn run(args: &[OsString]) -> io::Result<()> {
    let options = parse(args)?;
    engine_executable::executable_tables::initialize(&options.game_dir)?;
    let started = Instant::now();
    let language = stacks::language(&options.user_dir)?;
    let decode = content_reader_settings::settings::read_keys(&options.user_dir)?.texture_decode();
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
    if options.values.is_none() && value_dir.is_dir() {
        values::Output::clear(&value_dir)?;
    }
    let values = options.values.map(|_| values::Output::prepare(&value_dir)).transpose()?;
    let scratch = options.out.join("scratch");
    let cache = options
        .cache
        .clone()
        .unwrap_or_else(|| scratch.join("hashes"));
    let queue = Mutex::new(plans.iter());
    let reports = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..options.jobs.min(plans.len()) {
            job_priority::spawn_in(scope, || {
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
                        values.as_ref(),
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
    if let Some(error) = values.as_ref().and_then(|values| values.take_error()) {
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

fn stack(
    options: &Options,
    (language, decode): (&str, content_reader_image::native::Decode),
    candidates: &[stacks::Candidate],
    plan: &stacks::Plan,
    scratch: &Path,
    cache: &Path,
    output: Option<&Arc<values::Output>>,
) -> report::StackReport {
    let mut report = report::StackReport {
        stack: plan.id.clone(),
        name: plan.name.clone(),
        skipped_dependencies: plan.skipped.clone(),
        ..Default::default()
    };
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.id == plan.id) {
        report.descriptor_file = Some(candidate.file.clone());
        if let Ok(descriptor) = content_reader_parser::parser::parse(candidate.descriptor.as_bytes()) {
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
    let gathered = stacks::user_folder(&scratch.join(&plan.id), language, candidates, &plan.mods)
        .and_then(|user_dir| {
            content_reader_layers::gather(&content_reader_layers::GatherOptions {
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
    let values = output.zip(options.values).map(|(output, mode)| values::Writer::new(output, &plan.id, mode));
    let (runs, diagnostics, state_history) = if options.sounds_only {
        readers::run_sounds(content, values.clone())
    } else {
        readers::run(content, decode, values.clone(), options.maps_only)
    };
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
