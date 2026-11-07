//! Recording upgrades. Each step adds to a recording lacking one change of the recording format
//! what it lacks, losing nothing it held: the steps run in order, each added as a revision of the
//! recording in place, after which the revision before stays in the file.

use std::{
    io,
    path::{Path, PathBuf},
};

use installed_content_core::content::{ResolveOptions, facts, proof};
use replay::campaign::{CampaignWriter, Replay};

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
        /// One line per step taken.
        steps: Vec<String>,
    },
}

/// One change of the recording format, made losslessly: what it adds, and a line saying so.
struct Step {
    name: &'static str,
    applies: fn(&Replay) -> bool,
    add: fn(&Replay, &Path, &Folders) -> io::Result<String>,
}

const STEPS: &[Step] = &[Step {
    name: "game facts",
    applies: |replay| replay.fact_packs.is_empty(),
    add: add_facts,
}];

/// Upgrades the recording at `path` in place.
pub fn upgrade(path: &Path, folders: &Folders) -> io::Result<Outcome> {
    let opened = Replay::open_path(path)?;
    if STEPS.iter().all(|step| !(step.applies)(&opened)) {
        return Ok(Outcome::Current);
    }
    uncontinued(path, &opened)?;
    drop(opened);
    // No save adds to it meanwhile: every save holds the folder's naming lock.
    let folder = std::fs::canonicalize(path)?;
    let folder = folder.parent().unwrap_or_else(|| Path::new("."));
    let _names = replay::recording_files::NamesLock::wait(folder)?;
    let mut steps = Vec::new();
    for step in STEPS {
        let replay = Replay::open_path(path)?;
        uncontinued(path, &replay)?;
        if !(step.applies)(&replay) {
            continue;
        }
        let summary = (step.add)(&replay, path, folders)
            .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", step.name)))?;
        steps.push(format!("{}: {summary}", step.name));
    }
    Ok(if steps.is_empty() {
        Outcome::Current
    } else {
        Outcome::Upgraded { steps }
    })
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

/// Adds the game facts the recording depends on, proven layer by layer from whichever game
/// folders still hold each as recorded; facts no folder holds are withheld by name.
fn add_facts(replay: &Replay, path: &Path, folders: &Folders) -> io::Result<String> {

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
    let level = replay::recorder_settings::RecorderSettings::load()
        .map(|settings| settings.compression)
        .unwrap_or_default()
        .level();
    let mut writer = CampaignWriter::resume(replay, path, level)?;
    let before = replay.revision();
    if let Err(error) = writer
        .add_facts(&pack)
        .and_then(|()| writer.revise(|file| file.sync_data()))
    {
        drop(writer);
        let _ = replay::campaign::writer::roll_back(path, before);
        return Err(error);
    }
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
