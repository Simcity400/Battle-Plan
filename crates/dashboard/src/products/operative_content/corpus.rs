//! Opt-in, read-only installed-content audit. Its generated launcher metadata,
//! hash cache and report live only in the explicitly supplied scratch folder.

use super::*;
use installed_content::content::{GatherOptions, gather};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name)
            .unwrap_or_else(|| panic!("Set {name} explicitly for the installed-content audit")),
    )
}

fn audit(options: &GatherOptions) -> io::Result<Value> {
    println!("Audit phase: gather recorded content layers");
    let gathered = gather(options)?;
    let mut result = audit_layers(&gathered.layers)?;
    result["layers"] = json!(
        gathered
            .layers
            .layers
            .iter()
            .map(|layer| layer.id.as_str())
            .collect::<Vec<_>>()
    );
    result["dlc"] = json!(gathered.dlc.len());
    result["mods"] = json!(
        gathered
            .mods
            .iter()
            .map(|entry| &entry.display_name)
            .collect::<Vec<_>>()
    );
    result["orderNote"] = json!(gathered.order_note);
    result["contentFilesRead"] = json!(gathered.stats.files_read);
    Ok(result)
}

fn audit_layers(layers: &installed_content::ResolvedLayers) -> io::Result<Value> {
    println!("Audit phase: read operation metadata");
    let operations = installed_content::intelligence::read(layers)?;
    println!("Audit phase: read operative skill definitions");
    let leaders = Leaders::<String>::load(layers, &|_| None)?;
    println!("Audit phase: read mission text-icon declaration");
    let sprites = installed_content::gfx::read(layers)?;
    let mission_icon = catalogue::text_icon_sprite(&sprites, "operative_mission_icons_small")
        .map(|sprite| json!({"name": sprite.name, "frames": sprite.frames}))
        .map_err(|error| error.to_string());
    Ok(json!({
        "operationDefinitions": operations.operations.len(),
        "operationRefusals": operations.operation_refusals,
        "operationWarnings": operations.warnings,
        "operationNotes": operations.notes.iter()
            .map(|(kind, reason)| (format!("{kind:?}"), reason)).collect::<Vec<_>>(),
        "skillConditions": leaders.skills.conditions().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "skillWarnings": leaders.skills.warnings(),
        "skillAvailable": leaders.skills.next_required_experience(0, 3).is_some(),
        "missionIcon": mission_icon,
    }))
}

fn failed(entry: &&Value) -> bool {
    entry.get("error").is_some()
        || entry.get("result").is_some_and(|result| {
            !result["operationRefusals"].as_array().unwrap().is_empty()
                || !result["skillConditions"].as_array().unwrap().is_empty()
                || result["skillAvailable"] != true
                || result["missionIcon"].get("Err").is_some()
        })
}

fn descriptor_at(root: &Path) -> io::Result<Option<PathBuf>> {
    let descriptor = root.join("descriptor.mod");
    Ok(descriptor.is_file().then_some(descriptor))
}

#[test]
#[ignore = "Requires explicit external game/workshop/scratch/language paths; never runs a game"]
fn operative_content_native_reader_corpus() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let game = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_GAME");
    let workshop = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_WORKSHOP");
    let scratch = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_SCRATCH");
    let language = std::env::var("BATTLE_PLAN_OPERATIVE_CORPUS_LANGUAGE").or_else(|_| std::env::var("HOI4_OPERATIVE_CORPUS_LANGUAGE"))
        .expect("Set BATTLE_PLAN_OPERATIVE_CORPUS_LANGUAGE explicitly for the installed-content audit");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let game = game.canonicalize().unwrap();
    let workshop = workshop.canonicalize().unwrap();
    assert!(
        scratch.is_absolute(),
        "Audit scratch must be an absolute path"
    );
    // Validate an existing destination before the audit writes anything.
    let scratch = scratch
        .canonicalize()
        .expect("Create the external audit scratch directory first");
    assert!(
        !scratch.starts_with(&workspace),
        "Audit scratch must remain outside the worktree"
    );
    assert!(
        !scratch.starts_with(&game) && !scratch.starts_with(&workshop),
        "Audit scratch cannot be in read-only installed content"
    );
    let user = scratch.join("corpus-user");
    fs::create_dir_all(user.join("mod")).unwrap();
    fs::write(
        user.join("settings.txt"),
        format!("language={}\n", serde_json::to_string(&language).unwrap()),
    )
    .unwrap();
    let options = GatherOptions {
        game_dir: game,
        user_dir: user.clone(),
        cache_dir: scratch.join("corpus-cache"),
        dlc_order: None,
    };
    fs::write(
        user.join("dlc_load.json"),
        br#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    )
    .unwrap();
    let mut stacks = Vec::new();
    let base = audit(&options)
        .map(|result| json!({"label": "vanilla and installed DLC", "result": result}))
        .unwrap_or_else(
            |error| json!({"label": "vanilla and installed DLC", "error": error.to_string()}),
        );
    println!("{}", serde_json::to_string(&base).unwrap());
    stacks.push(base);
    let mut roots: Vec<_> = fs::read_dir(&workshop)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    roots.sort();
    let mut installed = 0;
    for root in roots {
        let Some(descriptor) = descriptor_at(&root).unwrap() else {
            continue;
        };
        installed += 1;
        let source = fs::read_to_string(&descriptor).unwrap();
        let parsed = installed_content::parser::parse(source.as_bytes()).unwrap();
        // Preserve all authored metadata, including replace_path and
        // dependencies. The injected first path identifies only the physical
        // read-only input; it does not change the source files.
        let physical = if let Some(archive) = parsed.get(b"archive") {
            let archive = archive.read_str().unwrap();
            format!(
                "archive={}\n",
                serde_json::to_string(&root.join(archive)).unwrap()
            )
        } else {
            format!("path={}\n", serde_json::to_string(&root).unwrap())
        };
        fs::write(user.join("mod/corpus.mod"), physical + &source).unwrap();
        fs::write(
            user.join("dlc_load.json"),
            br#"{"enabled_mods":["mod/corpus.mod"],"disabled_dlcs":[]}"#,
        )
        .unwrap();
        let label = root.file_name().unwrap().to_string_lossy();
        let entry = audit(&options)
            .map(|result| json!({"label": label, "result": result}))
            .unwrap_or_else(|error| json!({"label": label, "error": error.to_string()}));
        println!("{}", serde_json::to_string(&entry).unwrap());
        stacks.push(entry);
    }
    let failed = stacks.iter().filter(failed).count();
    let report = json!({"installedWorkshopDescriptors": installed, "stacks": stacks, "failedStacks": failed});
    fs::write(
        scratch.join("native-reader-corpus.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        installed > 0,
        "No Workshop descriptors were found in the supplied corpus"
    );
    assert_eq!(
        failed, 0,
        "Native-reader corpus has refused stacks; see external native-reader-corpus.json"
    );
}

#[derive(Default)]
struct Copied {
    files: u64,
    bytes: u64,
}

fn copy_directory(source: &Path, destination: &Path, copied: &mut Copied) -> io::Result<()> {
    let mut entries = fs::read_dir(source)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    fs::create_dir_all(destination)?;
    for entry in entries {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Script audit refuses a symbolic link: {}",
                    entry.path().display()
                ),
            ));
        }
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_directory(&entry.path(), &target, copied)?;
        } else if kind.is_file() {
            copied.bytes += fs::copy(entry.path(), target)?;
            copied.files += 1;
        }
    }
    Ok(())
}

fn mirror(source: &Path, destination: &Path) -> io::Result<Copied> {
    let mut copied = Copied::default();
    fs::create_dir_all(destination)?;
    // Exact bytes and relative spellings of all reader dependencies, including
    // common/synchronized_dynamic_tokens for the native interface sprite reader.
    for directory in ["common", "interface"] {
        let root = source.join(directory);
        if root.try_exists()? {
            copy_directory(&root, &destination.join(directory), &mut copied)?;
        }
    }
    Ok(copied)
}

#[test]
#[ignore = "Reads installed define scripts into explicit external scratch; no game, assets or DLC identity audit"]
fn operative_capture_define_reader_corpus() {
    let game = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_GAME")
        .canonicalize()
        .unwrap();
    let workshop = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_WORKSHOP")
        .canonicalize()
        .unwrap();
    let scratch = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_SCRATCH")
        .canonicalize()
        .unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    assert!(
        !scratch.starts_with(&workspace)
            && !scratch.starts_with(&game)
            && !scratch.starts_with(&workshop)
    );
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mirrors = scratch.join(format!(
        "capture-defines-inputs-{}-{unique}",
        std::process::id()
    ));
    let base = mirrors.join("vanilla");
    copy_directory(
        &game.join("common/defines"),
        &base.join("common/defines"),
        &mut Copied::default(),
    )
    .unwrap();
    let read = |label: &str, extra: Option<(&Path, &[&str])>| {
        let mut inputs = vec![(base.as_path(), &[][..])];
        inputs.extend(extra);
        let result = installed_content::ResolvedLayers::from_folders(&inputs)
            .map_err(|error| error.to_string())
            .and_then(|layers| capture_duration_days(&layers));
        json!({"label": label, "captureDurationDays": result})
    };
    let mut stacks = vec![read("vanilla scripts", None)];
    let mut roots = fs::read_dir(&workshop)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    roots.sort();
    for root in roots {
        let Some(descriptor) = descriptor_at(&root).unwrap() else {
            continue;
        };
        let label = root.file_name().unwrap().to_str().unwrap();
        let entry = (|| -> io::Result<Value> {
            let bytes = fs::read(&descriptor)?;
            let parsed = installed_content::parser::parse(&bytes)?;
            if parsed.get(b"archive").is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Capture define corpus refuses archive {}",
                        descriptor.display()
                    ),
                ));
            }
            let replacements = parsed
                .get_all(b"replace_path")
                .map(|value| value.read_str().map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason.to_string()))?;
            let replacement_refs = replacements.iter().map(String::as_str).collect::<Vec<_>>();
            let target = mirrors.join(label);
            fs::create_dir_all(&target)?;
            let source = root.join("common/defines");
            if source.try_exists()? {
                copy_directory(
                    &source,
                    &target.join("common/defines"),
                    &mut Copied::default(),
                )?;
            }
            Ok(read(label, Some((&target, &replacement_refs))))
        })()
        .unwrap_or_else(|error| json!({"label": label, "error": error.to_string()}));
        stacks.push(entry);
    }
    let refused = stacks
        .iter()
        .filter(|entry| {
            entry.get("error").is_some() || entry["captureDurationDays"].get("Err").is_some()
        })
        .count();
    let report = json!({"game": game, "workshop": workshop, "mirrors": mirrors, "stacks": stacks, "refusedStacks": refused, "limitations": ["No DLC layers or arbitrary multi-mod combinations", "No archive mount or recording identity verification", "Only capture define reader audited"]});
    fs::write(
        scratch.join("capture-define-reader-corpus.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
    assert!(stacks.len() > 1);
    assert_eq!(refused, 0, "See external capture-define-reader-corpus.json");
}

fn reader_entry(
    label: &str,
    base: &Path,
    extra: Option<(&Path, &[&str])>,
    originals: Value,
) -> Value {
    let mut inputs = vec![(base, &[][..])];
    inputs.extend(extra);
    let result =
        installed_content::ResolvedLayers::from_folders(&inputs).and_then(|layers| audit_layers(&layers));
    match result {
        Ok(result) => json!({"label": label, "inputRoots": originals, "result": result}),
        Err(error) => json!({"label": label, "inputRoots": originals, "error": error.to_string()}),
    }
}

#[test]
#[ignore = "Copies exact installed scripts to explicit external scratch; does not run a game or audit DLC/recording identity"]
fn operative_content_script_reader_corpus() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let game = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_GAME")
        .canonicalize()
        .unwrap();
    let workshop = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_WORKSHOP")
        .canonicalize()
        .unwrap();
    let scratch = required_path("BATTLE_PLAN_OPERATIVE_CORPUS_SCRATCH")
        .canonicalize()
        .expect("Create external audit scratch first");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    assert!(
        !scratch.starts_with(&workspace)
            && !scratch.starts_with(&game)
            && !scratch.starts_with(&workshop),
        "Script audit scratch must remain outside worktree and read-only content"
    );
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mirrors = scratch.join(format!(
        "script-corpus-inputs-{}-{unique}",
        std::process::id()
    ));
    let base = mirrors.join("vanilla");
    println!("Script audit: copy vanilla common/interface bytes to external scratch");
    let base_copied = mirror(&game, &base).unwrap();
    let base_original = json!({"source": game, "mirroredFiles": base_copied.files, "mirroredBytes": base_copied.bytes, "replacePaths": []});
    let mut stacks = vec![reader_entry(
        "vanilla scripts",
        &base,
        None,
        json!([base_original.clone()]),
    )];
    println!("{}", stacks[0]);
    let mut roots: Vec<_> = fs::read_dir(&workshop)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    roots.sort();
    let mut installed = 0;
    for root in roots {
        let Some(descriptor) = descriptor_at(&root).unwrap() else {
            continue;
        };
        installed += 1;
        let label = root.file_name().unwrap().to_str().unwrap();
        println!("Script audit: copy Workshop {label} common/interface bytes");
        let entry = (|| -> io::Result<Value> {
            let source = fs::read(&descriptor)?;
            let parsed = installed_content::parser::parse(&source)?;
            if parsed.get(b"archive").is_some() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Script mirror audit refuses archive source in {}; use the full gather audit for native archive proof", descriptor.display())));
            }
            let replace_paths = parsed.get_all(b"replace_path").map(|value| value.read_str().map(str::to_owned)).collect::<Result<Vec<_>, _>>().map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason.to_string()))?;
            let replace_refs: Vec<_> = replace_paths.iter().map(String::as_str).collect();
            let target = mirrors.join(label);
            let copied = mirror(&root, &target)?;
            Ok(reader_entry(label, &base, Some((&target, &replace_refs)), json!([base_original.clone(), {"source": root, "descriptor": descriptor, "mirroredFiles": copied.files, "mirroredBytes": copied.bytes, "replacePaths": replace_paths}])))
        })().unwrap_or_else(|error| json!({"label": label, "error": error.to_string()}));
        println!("{}", entry);
        stacks.push(entry);
    }
    let refused_stacks = stacks.iter().filter(failed).count();
    let report = json!({
        "audit": "Production native script readers over byte-exact common/interface mirrors, vanilla plus each Workshop folder individually",
        "limitations": ["No integrated or selectable DLC layers", "No multi-mod stack ordering", "No native archive mount proof", "No localisation or texture decoding", "Subset hashes are not recording content identities"],
        "sourceReferences": "Reader paths/lines retain the original relative spelling and line content; inputRoots supplies each physical original root in stack order",
        "installedWorkshopDescriptors": installed, "mirrors": mirrors, "stacks": stacks, "failedStacks": refused_stacks,
    });
    fs::write(
        scratch.join("script-reader-corpus.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(installed > 0, "No Workshop descriptors found");
    assert_eq!(
        refused_stacks, 0,
        "Native script-reader corpus has refusals; see external script-reader-corpus.json"
    );
}
