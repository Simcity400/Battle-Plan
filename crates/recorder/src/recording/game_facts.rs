
use content_reader_layers::{LayerRole, content::{GatheredContent, facts}};
use replay_data::identity::Identity;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

pub(super) struct GameFacts {
    cancelled: Arc<AtomicBool>,
    packing: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<String, String>>>,
}

struct Packed(Arc<AtomicBool>);

impl Drop for Packed {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl GameFacts {
    pub(super) fn start(
        content: &GatheredContent,
        identity: &Identity,
        session: &Path,
        cache: &Path,
    ) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&cancelled);
        let packing = Arc::new(AtomicBool::new(true));
        let packed = Packed(Arc::clone(&packing));
        let layers = content.layers.clone();
        let withheld = changed_layers(content, identity);
        let identity = identity.clone();
        let file = session.join(replay::campaign::SESSION_FACTS_FILE);
        let cache = cache.to_owned();
        let thread = std::thread::Builder::new()
            .name("game facts".into())
            .spawn(move || {
                let _packed = packed;
                if packed_already(&file, &identity) {
                    return Ok("kept as the session packed them".to_owned());
                }
                let withheld = withheld?;
                job_priority::background_thread();
                job_priority::confined(|| {
                    pack(&layers, withheld, &identity, &cache, &file, &stopping)
                })
            })
            .ok();
        Self { cancelled, packing, thread }
    }

    pub(super) fn packing(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.packing)
    }

    pub(super) fn finish(mut self) -> Result<String, String> {
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .unwrap_or_else(|_| Err("the game facts worker panicked".to_owned())),
            None => Err("the game facts worker could not start".to_owned()),
        }
    }
}

impl Drop for GameFacts {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.cancelled.store(true, Ordering::Release);
            let _ = thread.join();
        }
    }
}

fn packed_already(file: &Path, identity: &Identity) -> bool {
    std::fs::File::open(file)
        .ok()
        .and_then(|mut file| replay::campaign::session_facts_place(&mut file, identity).ok())
        .flatten()
        .is_some()
}

fn changed_layers(
    content: &GatheredContent,
    identity: &Identity,
) -> Result<BTreeMap<usize, String>, String> {
    if content.language != identity.language {
        return Err(format!(
            "the game's language is {} now, and the session recorded {}",
            content.language, identity.language
        ));
    }
    if !content
        .dlc
        .iter()
        .map(|dlc| &dlc.id)
        .eq(identity.dlc.iter().map(|dlc| &dlc.id))
    {
        return Err("the DLC installed now are not those the session recorded".to_owned());
    }
    if !content
        .mods
        .iter()
        .map(|layer| &layer.id)
        .eq(identity.mods.iter().map(|layer| &layer.id))
    {
        return Err("the mods enabled now are not those the session recorded".to_owned());
    }
    let changed = |kind: &str, id: &str, now: [u8; 32], then: [u8; 32]| {
        (now != then).then(|| format!("{kind} {id} changed since the session began"))
    };
    let mut withheld = BTreeMap::new();
    for (index, layer) in content.layers.layers.iter().enumerate() {
        let why = match layer.role {
            LayerRole::Dlc => content
                .dlc
                .iter()
                .zip(&identity.dlc)
                .find(|(now, _)| now.id == layer.id)
                .and_then(|(now, then)| changed("DLC", &now.id, now.hash, then.hash)),
            LayerRole::Mod => content
                .mods
                .iter()
                .zip(&identity.mods)
                .find(|(now, _)| now.id == layer.id)
                .and_then(|(now, then)| changed("Mod", &now.id, now.hash, then.hash)),
            LayerRole::Vanilla | LayerRole::Builtin | LayerRole::AboveMods => {
                (content.vanilla_hash != identity.vanilla_hash)
                    .then(|| "the base game changed since the session began".to_owned())
            }
            LayerRole::UserDirectory | LayerRole::Visual => None,
        };
        if let Some(why) = why {
            withheld.insert(index, why);
        }
    }
    Ok(withheld)
}

fn pack(
    layers: &content_reader_layers::ResolvedLayers,
    withheld: BTreeMap<usize, String>,
    identity: &Identity,
    cache: &Path,
    file: &PathBuf,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let source = facts::Source { layers, withheld };
    let executable = engine_executable::executable_tables::recorded_facts();
    let (pack, stats) = facts::write(&source, identity, executable, cache, &|| {
        cancelled.load(Ordering::Acquire)
    }, content_reader_key_names::equipment_facts::gather)
    .map_err(|error| error.to_string())?;
    let bytes =
        replay::campaign::encode_session_facts(identity, &pack).map_err(|e| e.to_string())?;
    let staged = file.with_extension("bin.tmp");
    std::fs::write(&staged, &bytes)
        .and_then(|()| std::fs::rename(&staged, file))
        .map_err(|error| format!("{}: {error}", file.display()))?;
    Ok(format!(
        "{} files, {:.1} MB, carried in {:.1} MB ({} of {} chunks reused)",
        stats.files,
        stats.raw_bytes as f64 / 1e6,
        stats.pack_bytes as f64 / 1e6,
        stats.cached_chunks,
        stats.chunks
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_layers::{GatherOptions, gather};
    use std::{fs, sync::Arc};

    #[test]
    fn a_take_over_withholds_a_changed_mod_and_keeps_facts_already_packed() {
        let root = crate::test_support::allocation_test_directory("take-over-changed-mod-facts");
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("game/hoi4.exe", "synthetic executable");
        write("game/common/ideas/base.txt", "ideas = { base = {} }");
        fs::create_dir_all(root.join("game/dlc")).unwrap();
        write("user/settings.txt", "language=l_english");
        write(
            "user/dlc_load.json",
            r#"{"enabled_mods":["mod/changed.mod"],"disabled_dlcs":[]}"#,
        );
        write("user/mod/changed.mod", r#"name="Changed" path="changed""#);
        write(
            "user/changed/common/ideas/mod.txt",
            "ideas = { first = {} }",
        );
        let options = GatherOptions {
            game_dir: root.join("game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
            dlc_order: None,
        };
        let began = gather(&options).unwrap();
        let identity = began
            .identity("build".into(), [1; 32], 20, "test".into(), [2; 32])
            .unwrap();
        write(
            "user/changed/common/ideas/mod.txt",
            "ideas = { second = {} }",
        );
        let now = gather(&options).unwrap();
        let session = root.join("session");
        fs::create_dir_all(&session).unwrap();
        let file = session.join(replay::campaign::SESSION_FACTS_FILE);
        GameFacts::start(&now, &identity, &session, &options.cache_dir)
            .finish()
            .unwrap();
        let packed = fs::read(&file).unwrap();
        let pack = replay::campaign::session_facts_for(&packed, &identity).unwrap();
        let pack = facts::Pack::open(Arc::new(pack.to_vec()), 0, pack.len() as u64).unwrap();
        let layer = |role| pack.layers.iter().find(|layer| layer.role == role).unwrap();
        assert_eq!(layer(LayerRole::Vanilla).withheld(), None);
        let changed = layer(LayerRole::Mod);
        let reason = changed
            .withheld()
            .expect("the changed mod's facts were carried");
        assert!(reason.contains("changed since"), "{reason}");
        GameFacts::start(&began, &identity, &session, &options.cache_dir)
            .finish()
            .unwrap();
        assert_eq!(fs::read(&file).unwrap(), packed);
        fs::remove_dir_all(root).unwrap();
    }
}
