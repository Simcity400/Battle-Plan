use dashboard::{
    prepared::pipeline::cost::{self, Options},
    products::ContentOptions,
};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const DEFAULT_RECORDING: &str =
    r"captures\lane-perf\recording\longrec117.battleplan";
const CAPTURES: &str = r"captures";
const ROOT: &str = r"captures\w1b-cost";
const RUNTIME_ROOT: &str = r"captures\w1b-runtime\self";
const USAGE: &str = concat!(
    "Usage: preparation-cost --mode decode|decode-sequential|families|runtime ",
    "[--recording PATH] [--output NEW-EMPTY-DIRECTORY] [--cores N] [--scopes-off] ",
    "[--file-cache uncontrolled|warm|operator-cold]\n",
    "Output must be under captures/w1b-cost or captures/w1b-runtime/self (canonical exact roots).\n",
    "Runtime defaults to captures/w1b-runtime/self; other modes default to captures/w1b-cost.\n",
    "Decode floors require --cores 1; runtime permits --cores 16 when available.\n",
    "Runtime is a PARTIAL main-store borrowed-extract measurement, not whole-app one-decode runtime. ",
    "All registered families, coordinator, side jobs, main encoders, store and publication cadence remain; ",
    "no S3, byte credits, frozen roots or spools."
);

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Lock {
    path: PathBuf,
    file: Option<File>,
}
impl Drop for Lock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

// No work waits for anything (AGENTS.md): the lock only keeps two runs out of one output.
fn lock(output: &Path) -> io::Result<Lock> {
    let path = output.join("MEASURE_RUNNING");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok(Lock {
        path,
        file: Some(file),
    })
}

fn write_failure(output: &Path, error: &io::Error) {
    let mut causes = Vec::new();
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = cause {
        causes.push(current.to_string());
        cause = current.source();
    }
    let failure = serde_json::json!({
        "failed":true,"error":error.to_string(),"error_chain":causes,
        "error_debug":format!("{error:?}"),"error_kind":format!("{:?}", error.kind()),
        "raw_os_error":error.raw_os_error()
    });
    let result = File::create(output.join("failure.json"))
        .and_then(|file| serde_json::to_writer_pretty(file, &failure).map_err(io::Error::from));
    if let Err(failure_error) = result {
        eprintln!("preparation-cost: could not write failure.json: {failure_error}");
    }
}

fn run() -> io::Result<()> {
    let mut recording = env::var_os("BATTLE_PLAN_COST_RECORDING").or_else(|| env::var_os("HOI4_COST_RECORDING"))
        .map(PathBuf::from)
        .unwrap_or_else(|| DEFAULT_RECORDING.into());
    let mut mode = "families".to_owned();
    let mut output = None;
    let mut scopes = true;
    let mut cores = 1usize;
    let mut file_cache = "uncontrolled".to_owned();
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            return Ok(());
        }
        if arg == "--scopes-off" {
            scopes = false;
            continue;
        }
        let value = args.next().ok_or_else(|| invalid("Missing option value"))?;
        match arg.to_str() {
            Some("--recording") => recording = value.into(),
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--mode") => mode = value.to_string_lossy().into_owned(),
            Some("--cores") => {
                cores = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| invalid("Invalid core count"))?
            }
            Some("--file-cache") => file_cache = value.to_string_lossy().into_owned(),
            _ => return Err(invalid(USAGE)),
        }
    }
    if !["decode", "decode-sequential", "families", "runtime"].contains(&mode.as_str()) {
        return Err(invalid(USAGE));
    }
    if !["uncontrolled", "warm", "operator-cold"].contains(&file_cache.as_str()) {
        return Err(invalid("Invalid file-cache label"));
    }
    if !recording.is_file() {
        return Err(invalid(format!(
            "Prerequisite failed: recording {} is missing; set BATTLE_PLAN_COST_RECORDING or --recording (default {DEFAULT_RECORDING})",
            recording.display()
        )));
    }
    let game = env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| env::var_os("HOI4_GAME_DIR"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            invalid("Prerequisite failed: set BATTLE_PLAN_GAME_DIR to the installed game directory")
        })?;
    let user = env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| env::var_os("HOI4_USER_DIR"))
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .map(|p| PathBuf::from(p).join("Documents/Paradox Interactive/Hearts of Iron IV"))
        })
        .ok_or_else(|| {
            invalid("Prerequisite failed: set BATTLE_PLAN_USER_DIR to the game's user folder")
        })?;
    if !game.join("hoi4.exe").is_file() || !user.is_dir() {
        return Err(invalid(
            "Prerequisite failed: BATTLE_PLAN_GAME_DIR must contain hoi4.exe and BATTLE_PLAN_USER_DIR must exist",
        ));
    }
    let root = if mode == "runtime" {
        RUNTIME_ROOT
    } else {
        ROOT
    };
    fs::create_dir_all(root)?;
    let root = fs::canonicalize(root)?;
    let output = output.unwrap_or_else(|| root.join(format!("{mode}-{}", uuid::Uuid::new_v4())));
    let parent = fs::canonicalize(
        output
            .parent()
            .ok_or_else(|| invalid("Output needs a parent"))?,
    )?;
    let captures = fs::canonicalize(CAPTURES)?;
    let roots = [
        captures.join("w1b-cost"),
        captures.join("w1b-runtime").join("self"),
    ];
    if !roots.iter().any(|root| parent.starts_with(root)) {
        return Err(invalid(
            "Output must stay under captures/w1b-cost or captures/w1b-runtime/self (canonical exact roots)",
        ));
    }
    let output = parent.join(
        output
            .file_name()
            .ok_or_else(|| invalid("Output needs a new directory name"))?,
    );
    fs::create_dir(&output).map_err(|error| {
        invalid(format!(
            "Output must be a new empty directory: {}: {error}",
            output.display()
        ))
    })?;
    let output = fs::canonicalize(output)?;
    if !roots.iter().any(|root| output.starts_with(root)) {
        return Err(invalid(
            "Canonical output escaped the allowed capture roots",
        ));
    }
    let result = (|| -> io::Result<()> {
        let _lock = lock(&output)?;
        let local = output.join("cache/local");
        let roaming = output.join("cache/roaming");
        fs::create_dir_all(&local)?;
        fs::create_dir_all(&roaming)?;
        // This executable sets isolation before starting any worker threads.
        unsafe {
            env::set_var("LOCALAPPDATA", &local);
            env::set_var("APPDATA", &roaming);
        }
        let options = Options {
            recording: fs::canonicalize(recording)?,
            output: output.clone(),
            mode,
            scopes,
            cores,
            file_cache,
            content: ContentOptions {
                options: installed_content_core::ResolveOptions {
                    game_dir: game,
                    user_dir: user,
                    cache_dir: output.join("cache/content"),
                },
                allow_unverified: false,
                presentation: Default::default(),
            },
        };
        fs::create_dir_all(&options.content.options.cache_dir)?;
        cost::run(&options)?;
        println!("{}", fs::read_to_string(output.join("summary.txt"))?);
        println!("Report: {}", output.join("report.json").display());
        Ok(())
    })();
    if let Err(error) = result {
        write_failure(&output, &error);
        return Err(error);
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("preparation-cost: {error}");
            ExitCode::FAILURE
        }
    }
}
