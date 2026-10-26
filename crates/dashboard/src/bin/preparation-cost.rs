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
    time::{Duration, SystemTime},
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const DEFAULT_RECORDING: &str =
    r"captures\lane-perf\recording\longrec117.battleplan";
const ROOT: &str = r"captures\w1b-cost";

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

fn test_lock() -> io::Result<()> {
    let path = Path::new(r"captures\ci\TESTS_RUNNING");
    match fs::metadata(path) {
        Ok(meta)
            if SystemTime::now()
                .duration_since(meta.modified()?)
                .unwrap_or_default()
                < Duration::from_secs(20 * 3600) =>
        {
            Err(invalid(format!(
                "Prerequisite failed: {} is active",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn lock(output: &Path) -> io::Result<Lock> {
    test_lock()?;
    let captures = Path::new(r"captures");
    let mut markers = vec![captures.join("BENCH_RUNNING")];
    for entry in fs::read_dir(captures)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            markers.push(entry.path().join("BENCH_RUNNING"));
        }
    }
    if let Some(active) = markers.into_iter().find(|path| path.exists()) {
        return Err(invalid(format!(
            "Prerequisite failed: {} exists",
            active.display()
        )));
    }
    let path = output.join("MEASURE_RUNNING");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let lock = Lock {
        path,
        file: Some(file),
    };
    test_lock()?;
    Ok(lock)
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
            _ => {
                return Err(invalid(
                    "Usage: preparation-cost --mode decode|decode-sequential|families [--recording PATH] [--output EMPTY-DIRECTORY-UNDER-w1b-cost] [--cores N] [--scopes-off] [--file-cache uncontrolled|warm|operator-cold]",
                ));
            }
        }
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
    fs::create_dir_all(ROOT)?;
    let root = fs::canonicalize(ROOT)?;
    let output = output.unwrap_or_else(|| root.join(format!("{mode}-{}", uuid::Uuid::new_v4())));
    let parent = fs::canonicalize(
        output
            .parent()
            .ok_or_else(|| invalid("Output needs a parent"))?,
    )?;
    if !parent.starts_with(&root) {
        return Err(invalid("Output must stay under captures/w1b-cost"));
    }
    fs::create_dir(&output).map_err(|error| {
        invalid(format!(
            "Output must be a new empty directory: {}: {error}",
            output.display()
        ))
    })?;
    let output = fs::canonicalize(output)?;
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
    match cost::run(&options) {
        Ok(_) => {
            println!("{}", fs::read_to_string(output.join("summary.txt"))?);
            println!("Report: {}", output.join("report.json").display());
            Ok(())
        }
        Err(error) => {
            serde_json::to_writer_pretty(
                File::create(output.join("failure.json"))?,
                &serde_json::json!({"failed":true,"error":error.to_string()}),
            )?;
            Err(error)
        }
    }
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
