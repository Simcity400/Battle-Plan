//! Recording upgrades. Each step takes a recording lacking one change of the recording format
//! to a copy that has it, losing nothing it held; the steps run in order, and the original is
//! kept in a `.originals` folder beside it, which the recording library does not list.

use std::{
    io,
    path::{Path, PathBuf},
};

use installed_content_core::content::{ResolveOptions, facts, proof};
use replay::container::Replay;

/// Where an upgrade looks for what a recording needs: game folders, newest first, which may
/// hold older versions of the game, and the user folder whose mods they mount.
#[derive(Clone, Debug)]
pub struct Folders {
    pub games: Vec<PathBuf>,
    pub user: PathBuf,
    pub cache: PathBuf,
}

/// What upgrading a recording did.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// It has every change already.
    Current,
    Upgraded {
        /// Where the original now is.
        original: PathBuf,
        /// One line per step taken.
        steps: Vec<String>,
    },
}

/// One change of the recording format, made losslessly.
struct Step {
    name: &'static str,
    applies: fn(&Replay) -> bool,
    write: fn(&Replay, &Path, &Folders) -> io::Result<String>,
}

const STEPS: &[Step] = &[Step {
    name: "game facts",
    applies: |replay| replay.fact_packs.is_empty(),
    write: add_facts,
}];

/// Upgrades the recording at `path` in place, keeping the original.
pub fn upgrade(path: &Path, folders: &Folders) -> io::Result<Outcome> {
    let opened = Replay::open_path(path)?;
    if STEPS.iter().all(|step| !(step.applies)(&opened)) {
        return Ok(Outcome::Current);
    }
    uncontinued(path, &opened)?;
    let before = opened.revision();
    drop(opened);
    let mut current = path.to_owned();
    let mut steps = Vec::new();
    let mut staged = Vec::new();
    let result = (|| {
        for (index, step) in STEPS.iter().enumerate() {
            let replay = Replay::open_path(&current)?;
            if !(step.applies)(&replay) {
                continue;
            }
            let out = staging(path, index);
            let _ = std::fs::remove_file(&out);
            let summary = (step.write)(&replay, &out, folders)
                .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", step.name)))?;
            steps.push(format!("{}: {summary}", step.name));
            staged.push(out.clone());
            current = out;
        }
        io::Result::Ok(())
    })();
    let finished = result.and_then(|()| {
        if steps.is_empty() {
            return Ok(Outcome::Current);
        }
        // No save starts an unsaved continuation of it, and no writer opens it, from the checks
        // to the replacement, so nothing saved meanwhile is replaced or left unable to save.
        let folder = std::fs::canonicalize(path)?;
        let folder = folder.parent().unwrap_or_else(|| Path::new("."));
        let _names = replay::recording_files::NamesLock::wait(folder)?;
        let _excluding_writers = exclude_writers(path)?;
        let now = Replay::open_path(path)?;
        if now.revision() != before {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the recording was saved while it was upgraded; upgrade it again",
            ));
        }
        uncontinued(path, &now)?;
        drop(now);
        let original = keep_original(path)?;
        if let Err((replaced, error)) = publish(&current, path) {
            // The original stays kept unless the recording certainly still holds it.
            if !replaced {
                let _ = std::fs::remove_file(&original);
            }
            return Err(error);
        }
        Ok(Outcome::Upgraded { original, steps })
    });
    for file in staged {
        if file != current || finished.is_err() {
            let _ = std::fs::remove_file(file);
        }
    }
    finished
}

/// Refuses a recording an unsaved recording is, or continues: that continuation must go on
/// holding it unchanged, so it is upgraded once its unsaved hours are saved or discarded.
fn uncontinued(path: &Path, replay: &Replay) -> io::Result<()> {
    // Its own folder, however `path` was spelled.
    let path = std::fs::canonicalize(path)?;
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    let deferred = |why: String| Err(io::Error::new(io::ErrorKind::WouldBlock, why));
    if folder.file_name() == replay::publication::unsaved_folder(Path::new("")).file_name() {
        return deferred("unsaved hours are upgraded once they are saved".to_owned());
    }
    match replay::recording_files::unsaved_recording(folder, &replay.campaign_id) {
        Ok(None) => Ok(()),
        Ok(Some(_)) => deferred(
            "unsaved hours continue it; it is upgraded once they are saved or discarded".to_owned(),
        ),
        Err(error) => deferred(format!("its unsaved hours cannot be told apart ({error})")),
    }
}

fn staging(path: &Path, step: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".upgrade{step}.tmp"));
    path.with_file_name(name)
}

/// `staged` in `path`'s place in one step, so the recording is never absent from its path; on
/// failure, whether the replacement may have happened.
fn publish(staged: &Path, path: &Path) -> Result<(), (bool, io::Error)> {
    #[cfg(windows)]
    {
        use replay::publication::ReplaceError;
        replay::publication::replace_file(staged, path).map_err(|error| match error {
            ReplaceError::Unrenamed(error) => (false, error),
            ReplaceError::Unflushed(error) => (true, error),
        })
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(staged, path).map_err(|error| (false, error))
    }
}

/// The recording held open sharing reads and its replacement only, so no recorder can open it to
/// add a save.
fn exclude_writers(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_DELETE: u32 = 4;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
    }
    options.open(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("the recording is being written; upgrade it later ({error})"),
        )
    })
}

/// Keeps the original beside the recording in `.originals`, under a name no other holds, as a
/// second name of the same file, or a copy where the volume has no links.
fn keep_original(path: &Path) -> io::Result<PathBuf> {
    let folder = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".originals");
    std::fs::create_dir_all(&folder)?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut kept = folder.join(path.file_name().unwrap_or_default());
    let mut number = 1;
    while kept.exists() {
        number += 1;
        kept = folder.join(format!("{stem} ({number}).battleplan"));
    }
    if std::fs::hard_link(path, &kept).is_err() {
        std::fs::copy(path, &kept)?;
        std::fs::File::open(&kept)?.sync_all()?;
    }
    Ok(kept)
}

/// Adds the game facts the recording depends on, proven layer by layer from whichever game
/// folders still hold each as recorded; facts no folder holds are withheld by name.
fn add_facts(replay: &Replay, out: &Path, folders: &Folders) -> io::Result<String> {
    let identity = &replay.identity;
    let mut candidates = Vec::new();
    let mut unusable = Vec::new();
    for game in &folders.games {
        let options = ResolveOptions {
            game_dir: game.clone(),
            user_dir: folders.user.clone(),
            cache_dir: folders.cache.clone(),
        };
        match proof::candidate(identity, &options, &[]) {
            Ok(candidate) => candidates.push(candidate),
            Err(error) => unusable.push(format!("{}: {error}", game.display())),
        }
    }
    let (layers, withheld, executable) = proof::assemble(identity, &candidates)?;
    // A pack proving nothing would keep a later upgrade from proving what it can.
    if withheld.len() == layers.layers.len() && executable.is_none() {
        let mut why = vec!["no game folder holds any of its content as recorded".to_owned()];
        why.extend(withheld.into_values().take(1));
        why.extend(unusable);
        return Err(io::Error::new(io::ErrorKind::NotFound, why.join("; ")));
    }
    let tables = match &executable {
        Some(path) => {
            engine_executable::executable_tables::recorded_facts_from(path, identity.exe_hash)
        }
        None => Err("no game folder holds the recorded executable".to_owned()),
    };
    let source = facts::Source {
        layers: &layers,
        withheld,
    };
    let (pack, stats) = facts::write(&source, identity, tables, &folders.cache, &|| false)?;
    replay.write_with_facts(out, &pack)?;
    let opened = facts::Pack::open(std::sync::Arc::new(pack), 0, stats.pack_bytes)?;
    let mut summary = format!(
        "{} files, {:.1} MB, carried in {:.1} MB",
        stats.files,
        stats.raw_bytes as f64 / 1e6,
        stats.pack_bytes as f64 / 1e6
    );
    for layer in &opened.layers {
        if let Some(reason) = layer.withheld() {
            summary.push_str(&format!("; {} withheld: {reason}", layer.id));
        }
    }
    if let Err(reason) = &opened.executable {
        summary.push_str(&format!("; executable tables withheld: {reason}"));
    }
    for line in unusable {
        summary.push_str(&format!("; not used: {line}"));
    }
    Ok(summary)
}
