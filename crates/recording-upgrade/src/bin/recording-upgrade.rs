
use std::path::PathBuf;

fn main() {
    let mut games = Vec::new();
    let mut user = None;
    let mut cache = None;
    let mut targets = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--game") => games.extend(args.next().map(PathBuf::from)),
            Some("--user") => user = args.next().map(PathBuf::from),
            Some("--cache") => cache = args.next().map(PathBuf::from),
            _ => targets.push(PathBuf::from(arg)),
        }
    }
    let Some(user) = user else {
        eprintln!("usage: recording-upgrade --game <folder>... --user <folder> [--cache <folder>] <recording or folder>...");
        std::process::exit(2);
    };
    let folders = recording_upgrade::Folders {
        games,
        user,
        cache: cache.unwrap_or_else(replay_settings::recorder_settings::directory),
    };
    let mut recordings = Vec::new();
    for target in targets {
        collect(&target, &mut recordings);
    }
    let mut failed = false;
    for recording in recordings {
        match recording_upgrade::upgrade(&recording, &folders) {
            Ok(recording_upgrade::Outcome::Current) => {
                println!("{}: current", recording.display())
            }
            Ok(recording_upgrade::Outcome::Upgraded { steps }) => {
                println!("{}: upgraded", recording.display());
                for step in steps {
                    println!("  {step}");
                }
            }
            Err(error) => {
                failed = true;
                println!("{}: not upgraded: {error}", recording.display());
            }
        }
    }
    std::process::exit(i32::from(failed));
}

fn collect(path: &std::path::Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|entry| entry.path()).collect();
        entries.sort();
        for entry in entries {
            if entry
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            collect(&entry, out);
        }
    } else if replay_files::recording_files::is_recording(path) {
        out.push(path.to_owned());
    }
}
