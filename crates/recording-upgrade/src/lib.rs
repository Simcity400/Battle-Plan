
use std::{
    io,
    path::{Path, PathBuf},
};

use content_reader_core::content::{ResolveOptions, facts, proof};
use replay::campaign::{Replay, writer};

const SHARING_VIOLATION: i32 = 32;

#[derive(Clone, Debug)]
pub struct Folders {
    pub games: Vec<PathBuf>,
    pub user: PathBuf,
    pub cache: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Current,
    Upgraded {
        steps: Vec<String>,
    },
}

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

pub fn upgrade(path: &Path, folders: &Folders) -> io::Result<Outcome> {
    let opened = Replay::open_newest(path)?;
    if STEPS.iter().all(|step| !(step.applies)(&opened)) {
        return Ok(Outcome::Current);
    }
    uncontinued(path, &opened)?;
    drop(opened);
    let folder = std::fs::canonicalize(path)?;
    let folder = folder.parent().unwrap_or_else(|| Path::new("."));
    let _names = replay_files::recording_files::NamesLock::wait(folder)?;
    let mut steps = Vec::new();
    for step in STEPS {
        let replay = Replay::open_newest(path)?;
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

fn uncontinued(path: &Path, replay: &Replay) -> io::Result<()> {
    let path = std::fs::canonicalize(path)?;
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    let deferred = |why: &str| Err(io::Error::new(io::ErrorKind::WouldBlock, why));
    if folder.file_name() == replay_settings::publication::unsaved_folder(Path::new("")).file_name() {
        return deferred("unsaved hours are upgraded once they are saved");
    }
    if replay.unsaved() {
        return deferred("it holds unsaved hours; it is upgraded once they are saved or discarded");
    }
    Ok(())
}

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
    let level = replay_settings::recorder_settings::RecorderSettings::load()
        .map(|settings| settings.compression)
        .unwrap_or_default()
        .level();
    let addition = writer::Addition {
        facts: Some(&pack),
        ..writer::Addition::default()
    };
    writer::revise(path, replay.recording(), &addition, true, level).map_err(|error| match error.raw_os_error() {
        Some(SHARING_VIOLATION) => io::Error::new(
            io::ErrorKind::WouldBlock,
            "a recorder is writing it; it is upgraded once the game has left it",
        ),
        _ => error,
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_core::{GatherOptions, gather};
    use replay_capture_spool::spool::Record;
    use replay_data::{identity::CaptureMode, identity::SampleBoundary, identity::SessionStart, rows::CampaignRow, rows::TableId, rows::TableRows};
    use replay_frame::{branches::BranchReason, branches::BranchStart, branches::Parentage, frame::Frame, frame::Metrics, frame::TableUpdate};
    use std::{collections::BTreeMap, fs, sync::Arc};

    const CAMPAIGN: &[u8] = b"upgrade-test";

    fn unequipped(pack: &[u8]) -> Vec<u8> {
        let why = facts::Pack::open(Arc::new(pack.to_vec()), 0, pack.len() as u64).unwrap().sub_unit_needs().unwrap_err();
        let compressed = u32::from_le_bytes(pack[6..10].try_into().unwrap()) as usize;
        let raw = u32::from_le_bytes(pack[10..14].try_into().unwrap()) as usize;
        let mut manifest = zstd::bulk::decompress(&pack[14..14 + compressed], raw).unwrap();
        manifest.truncate(manifest.len() - (1 + 4 + why.len()));
        let packed = zstd::bulk::compress(&manifest, 1).unwrap();
        let mut out = pack[..4].to_vec();
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        out.extend_from_slice(&pack[14 + compressed..]);
        out
    }

    #[test]
    fn a_recording_carrying_older_facts_keeps_every_layer_they_carry_when_a_mod_changed() {
        let root = std::env::temp_dir().join(format!("hoi4-upgrade-older-facts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("game/hoi4.exe", "synthetic executable");
        write("game/common/ideas/base.txt", "ideas = { base = {} }");
        fs::create_dir_all(root.join("game/dlc")).unwrap();
        write("user/settings.txt", "language=l_english");
        write("user/dlc_load.json", r#"{"enabled_mods":["mod/changed.mod"],"disabled_dlcs":[]}"#);
        write("user/mod/changed.mod", r#"name="Changed" path="changed""#);
        write("user/changed/common/ideas/mod.txt", "ideas = { first = {} }");
        let folders = Folders { games: vec![root.join("game")], user: root.join("user"), cache: root.join("cache") };
        let content = gather(&GatherOptions { game_dir: root.join("game"), user_dir: root.join("user"), cache_dir: folders.cache.clone(), dlc_order: None }).unwrap();
        let identity = content.identity("build".into(), [1; 32], 20, "test".into(), [2; 32]).unwrap();
        let source = facts::Source { layers: &content.layers, withheld: BTreeMap::new() };
        let (pack, _) = facts::write(&source, &identity, Err("no tables".to_owned()), &folders.cache, &|| false).unwrap();
        let older = unequipped(&pack);
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity,
        };
        let mut tables = std::array::from_fn(|_| TableUpdate::Unavailable { reason: "not read".into() });
        tables[TableId::Campaign.index()] = TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
        let records = vec![
            Record::SessionStart(session),
            Record::BranchStart(BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 100, parent: Parentage::Unknown, reason: BranchReason::SessionStart }),
            Record::Frame(Frame { branch: 1, hour: 100, keyframe: true, metrics: Metrics::default(), tables }),
        ];
        let path = root.join("recordings/synthetic.battleplan");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records]).unwrap().write_atomic(&path).unwrap();
        let recorded = Replay::open_newest(&path).unwrap();
        let addition = writer::Addition { facts: Some(&older), ..writer::Addition::default() };
        let level = replay_settings::compression::Compression::default().level();
        writer::revise(&path, recorded.recording(), &addition, true, level).unwrap();
        drop(recorded);
        write("user/changed/common/ideas/mod.txt", "ideas = { second = {} }");

        assert!(matches!(upgrade(&path, &folders).unwrap(), Outcome::Current));
        let kept = Replay::open_newest(&path).unwrap().newest_facts().unwrap().unwrap();
        assert_eq!(kept, older, "the facts the recording carried are still its newest");
        let kept = facts::Pack::open(Arc::new(kept), 0, older.len() as u64).unwrap();
        assert!(kept.layers.iter().all(|layer| layer.withheld().is_none()), "{:?}", kept.layers);
        fs::remove_dir_all(&root).unwrap();
    }
}
