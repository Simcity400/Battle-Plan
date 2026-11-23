
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use replay_identity::Identity;

use super::{
    CoveragePolicy, Descriptors, HASH_CACHE, HashCache, HashStats, LayerRole, LayerSource,
    ResolveOptions, ResolvedLayer, ResolvedLayers, builtin_layer, descriptor_layer,
    find_descriptors, layer_hash, mod_id, normalize_relative, publish_hash_cache,
    sort_descriptors, text_path,
};

pub struct Candidate {
    pub game_dir: PathBuf,
    pub game: Result<(Vec<ResolvedLayer>, Vec<ResolvedLayer>), String>,
    pub dlc: Vec<Result<ResolvedLayer, String>>,
    pub mods: Vec<Result<ResolvedLayer, String>>,
    pub executable: Option<PathBuf>,
}

pub fn candidate(
    identity: &Identity,
    options: &ResolveOptions,
    cancelled: &[&AtomicBool],
) -> io::Result<Candidate> {
    identity.validate()?;
    let coverage = CoveragePolicy::recorded(identity.coverage_policy)?;
    let game_dir = fs::canonicalize(&options.game_dir)?;
    let user_dir = fs::canonicalize(&options.user_dir).ok();
    let _ = fs::create_dir_all(&options.cache_dir);
    let cache_path = options.cache_dir.join(HASH_CACHE);
    let mut cache: HashCache = fs::read(&cache_path)
        .ok()
        .and_then(|bytes| HashCache::decode(&bytes))
        .unwrap_or_else(|| HashCache {
            dirty: true,
            ..HashCache::default()
        });
    cache.cancelled = cancelled;
    let mut stats = HashStats::default();
    let result = (|| {
        let executable = game_dir.join("hoi4.exe");
        let executable = (executable.is_file()
            && cache.file(&executable, &mut stats)? == identity.exe_hash)
            .then_some(executable);
        let game = match game_layers(&game_dir, coverage, &mut cache, &mut stats) {
            Ok((below, above, hash)) if hash == identity.vanilla_hash => Ok((below, above)),
            Ok(_) => Err("its base game differs from the recorded one".to_owned()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => Err(error.to_string()),
        };
        let descriptors = Descriptors::installed(Some(&game_dir), user_dir.as_deref());
        let mut dlc = Vec::new();
        for recorded in &identity.dlc {
            let layer = match descriptors.dlc.get(&recorded.id).map(Vec::as_slice) {
                Some([path]) => {
                    descriptor_layer(recorded.id.clone(), path.clone(), &game_dir, false, coverage)
                        .and_then(|(mut layer, _)| {
                            layer.inventory()?;
                            let hash = layer.hash(&mut cache, &mut stats)?;
                            Ok((layer, hash))
                        })
                }
                Some(_) => Err(super::invalid("several installed descriptors have its id")),
                None => Err(io::Error::new(io::ErrorKind::NotFound, "not installed")),
            };
            dlc.push(proven(layer, recorded.hash)?);
        }
        let mut mods = Vec::new();
        for recorded in &identity.mods {
            let layer = match (
                &user_dir,
                descriptors.mods.get(&recorded.id).map(Vec::as_slice),
            ) {
                (Some(user_dir), Some([path])) => {
                    descriptor_layer(recorded.id.clone(), path.clone(), user_dir, true, coverage)
                        .and_then(|(mut layer, document)| {
                            if mod_id(&document, path)? != recorded.id {
                                return Err(super::invalid("its descriptor names another mod"));
                            }
                            layer.inventory()?;
                            let hash = layer.hash(&mut cache, &mut stats)?;
                            Ok((layer, hash))
                        })
                }
                (_, Some([_, _, ..])) => {
                    Err(super::invalid("several installed descriptors have its id"))
                }
                _ => Err(io::Error::new(io::ErrorKind::NotFound, "not installed")),
            };
            mods.push(proven(layer, recorded.hash)?);
        }
        Ok(Candidate {
            game_dir: game_dir.clone(),
            game,
            dlc,
            mods,
            executable,
        })
    })();
    if cache.dirty {
        let _ = publish_hash_cache(&cache_path, cache);
    }
    result
}

fn proven(
    layer: io::Result<(ResolvedLayer, [u8; 32])>,
    recorded: [u8; 32],
) -> io::Result<Result<ResolvedLayer, String>> {
    Ok(match layer {
        Ok((layer, hash)) if hash == recorded => Ok(layer),
        Ok(_) => Err("the installed version differs from the recorded one".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => Err(error.to_string()),
    })
}

fn game_layers(
    game_dir: &Path,
    coverage: CoveragePolicy,
    cache: &mut HashCache,
    stats: &mut HashStats,
) -> io::Result<(Vec<ResolvedLayer>, Vec<ResolvedLayer>, [u8; 32])> {
    let mut vanilla = builtin_layer(
        LayerRole::Vanilla,
        "vanilla".into(),
        LayerSource::Folder(game_dir.to_owned()),
        None,
        coverage,
    )?;
    vanilla.inventory()?;
    vanilla.hash(cache, stats)?;
    let mut files = (*vanilla.hashes).clone();
    let mut below = vec![vanilla];
    let mut above = Vec::new();
    let mut add = |mut layer: ResolvedLayer,
                   prefix: &str,
                   cache: &mut HashCache,
                   stats: &mut HashStats|
     -> io::Result<ResolvedLayer> {
        layer.inventory()?;
        layer.hash(cache, stats)?;
        for (path, digest) in layer.hashes.iter() {
            files.insert([prefix.as_bytes(), b"/", path].concat(), *digest);
        }
        if let Some(descriptor) = &layer.descriptor {
            let name = descriptor
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| super::invalid("built-in descriptor has no Unicode name"))?;
            files.insert(
                format!("{prefix}/{name}").into_bytes(),
                layer.descriptor_hash.unwrap().bytes,
            );
        }
        Ok(layer)
    };
    let integrated_dir = game_dir.join("integrated_dlc");
    if integrated_dir.try_exists()? {
        let mut integrated = Vec::new();
        find_descriptors(&integrated_dir, &mut integrated)?;
        sort_descriptors(&mut integrated)?;
        for descriptor in integrated {
            let parent = descriptor
                .parent()
                .ok_or_else(|| super::invalid("integrated descriptor has no parent"))?
                .to_owned();
            let prefix = normalize_relative(text_path(
                parent
                    .strip_prefix(game_dir)
                    .map_err(|_| super::invalid("integrated content escaped game folder"))?,
            )?)?;
            let layer = builtin_layer(
                LayerRole::Builtin,
                prefix.clone(),
                LayerSource::Folder(parent),
                Some(descriptor),
                coverage,
            )?;
            below.push(add(layer, &prefix, cache, stats)?);
        }
    }
    let folder = game_dir.join("tweakergui_assets");
    let archive = game_dir.join("tweakergui_assets.zip");
    let tweaker = if folder.is_dir() {
        Some(("tweakergui_assets", LayerSource::Folder(folder)))
    } else if archive.is_file() {
        Some(("tweakergui_assets.zip", LayerSource::Archive(archive)))
    } else {
        None
    };
    if let Some((prefix, source)) = tweaker {
        let layer = builtin_layer(LayerRole::AboveMods, prefix.into(), source, None, coverage)?;
        above.push(add(layer, prefix, cache, stats)?);
    }
    let hash = layer_hash(&files)?;
    Ok((below, above, hash))
}

pub fn assemble(
    identity: &Identity,
    candidates: &[Candidate],
) -> io::Result<(ResolvedLayers, BTreeMap<usize, String>, Option<PathBuf>)> {
    let coverage = CoveragePolicy::recorded(identity.coverage_policy)?;
    let mut layers = ResolvedLayers {
        coverage,
        ..ResolvedLayers::default()
    };
    let mut withheld = BTreeMap::new();
    let why = |reasons: Vec<String>| {
        if reasons.is_empty() {
            "no game folder was given to prove it from".to_owned()
        } else {
            format!("no game folder holds it as recorded ({})", reasons.join("; "))
        }
    };
    let placeholder = |role: LayerRole, id: &str| ResolvedLayer {
        content_digest: [0; 32],
        inventory_witnesses: Default::default(),
        inventory_paths: Default::default(),
        coverage,
        role,
        facts: None,
        id: id.to_owned(),
        source: LayerSource::Folder(PathBuf::new()),
        descriptor: None,
        replace_paths: Vec::new(),
        files: Default::default(),
        lookup: Default::default(),
        hashes: Default::default(),
        archive_stamp: None,
        descriptor_hash: None,
        overlay: None,
    };
    let game = candidates
        .iter()
        .find_map(|candidate| candidate.game.as_ref().ok());
    let mut above = Vec::new();
    match game {
        Some((below, top)) => {
            layers.layers.extend(below.iter().cloned());
            above.extend(top.iter().cloned());
        }
        None => {
            let reasons = candidates
                .iter()
                .filter_map(|candidate| {
                    candidate
                        .game
                        .as_ref()
                        .err()
                        .map(|why| format!("{}: {why}", candidate.game_dir.display()))
                })
                .collect();
            withheld.insert(layers.layers.len(), why(reasons));
            layers.layers.push(placeholder(LayerRole::Vanilla, "vanilla"));
        }
    }
    let mut pick = |role: LayerRole,
                    id: &str,
                    found: &dyn Fn(&Candidate) -> &Result<ResolvedLayer, String>,
                    layers: &mut ResolvedLayers| {
        match candidates.iter().find_map(|candidate| found(candidate).as_ref().ok()) {
            Some(layer) => layers.layers.push(layer.clone()),
            None => {
                let reasons = candidates
                    .iter()
                    .filter_map(|candidate| {
                        found(candidate)
                            .as_ref()
                            .err()
                            .map(|why| format!("{}: {why}", candidate.game_dir.display()))
                    })
                    .collect();
                withheld.insert(layers.layers.len(), why(reasons));
                layers.layers.push(placeholder(role, id));
            }
        }
    };
    for (index, recorded) in identity.dlc.iter().enumerate() {
        pick(
            LayerRole::Dlc,
            &recorded.id,
            &|candidate| &candidate.dlc[index],
            &mut layers,
        );
    }
    for (index, recorded) in identity.mods.iter().enumerate() {
        pick(
            LayerRole::Mod,
            &recorded.id,
            &|candidate| &candidate.mods[index],
            &mut layers,
        );
    }
    layers.above_mods = above.len();
    layers.layers.extend(above);
    let executable = candidates
        .iter()
        .find_map(|candidate| candidate.executable.clone());
    Ok((layers, withheld, executable))
}
