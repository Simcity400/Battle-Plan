use super::*;
use crate::{SourceKey, test_fixtures::source};
use replay::container::Replay;
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn source_with_content(
    frames: Vec<replay_frame::frame::DecodedFrame>,
    root: &Path,
) -> (Source, LoadContext) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for settings metadata tests");
    crate::test_fixtures::source_with_content(frames, root)
}

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("hoi4-reply-cache-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}

fn reopen(source: &Source, path: &str) -> Source {
    Source::from_replay(
        SourceKey {
            source_id: path.into(),
            revision: path.into(),
        },
        path.into(),
        source.replay(),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn load(source: &Source, lease: &str, params: &Value, context: &LoadContext) -> Value {
    load_product("world-tension", source, lease, params, context)
}

fn load_product(
    key: &str,
    source: &Source,
    lease: &str,
    params: &Value,
    context: &LoadContext,
) -> Value {
    let product = crate::products::oracle_route(key).unwrap();
    let reply = (product.load)(
        source,
        lease,
        params.clone(),
        context,
        &AtomicBool::new(false),
    )
    .unwrap();
    serde_json::from_str(&reply).unwrap()
}

fn with_identity(identity: replay_data::identity::Identity) -> Source {
    let mut input = crate::test_fixtures::records(1, vec![source(1, 100, Some(1))]);
    let replay::spool::Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    from_bytes(
        Replay::from_records(crate::test_fixtures::CAMPAIGN.into(), &[input])
            .unwrap()
            .bytes()
            .to_vec(),
    )
}

fn from_bytes(bytes: Vec<u8>) -> Source {
    Source::from_replay(
        SourceKey {
            source_id: "fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::open(bytes).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn mod_source(root: &Path) -> (Source, LoadContext, PathBuf) {
    let (_, context) = source_with_content(vec![source(1, 100, Some(1))], root);
    let installed = root.join("installed-mod");
    fs::create_dir_all(installed.join("localisation")).unwrap();
    fs::create_dir_all(root.join("user/mod")).unwrap();
    let path = installed.join("localisation/mod_l_english.yml");
    fs::write(
        &path,
        "\u{feff}l_english:\n CASUALTIES_UNKNOWN:0 \"Mod casualty title\"\n",
    )
    .unwrap();
    fs::write(
        root.join("user/mod/cache-fixture.mod"),
        format!(
            "name=CacheFixture path=\"{}\"",
            installed.to_string_lossy().replace('\\', "/"),
        ),
    )
    .unwrap();
    fs::write(
        root.join("user/dlc_load.json"),
        r#"{"enabled_mods":["mod/cache-fixture.mod"],"disabled_dlcs":[]}"#,
    )
    .unwrap();
    let (_, identity) = crate::test_fixtures::content_identity(root);
    assert_eq!(identity.mods.len(), 1);
    (with_identity(identity), context, path)
}

#[test]
fn missing_recorded_mod_replies_are_live_and_not_persisted() {
    let root = root("missing-mod");
    let (first, context, path) = mod_source(&root);
    let params = json!({"branchId":"1", "tags":[], "top":null});
    let entry = Entry::new(&first, "war-casualties", true, &params, &context).unwrap();
    let descriptor = root.join("user/mod/cache-fixture.mod");
    let descriptor_bytes = fs::read(&descriptor).unwrap();
    fs::remove_file(&descriptor).unwrap();
    let missing = reopen(&first, "missing");
    let live = load_product("war-casualties", &missing, "missing", &params, &context);
    assert!(
        live["data"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("unavailable"))
    );
    assert!(entry.get().is_none(), "degraded replies must not persist");
    assert!(Entry::new(&missing, "war-casualties", true, &params, &context).is_none());
    // Recording-only products remain cacheable with the same unavailable mod.
    let numeric_params = json!({"branchId":"1"});
    load(&missing, "numeric", &numeric_params, &context);
    assert!(
        Entry::new(&missing, "world-tension", false, &numeric_params, &context)
            .unwrap()
            .get()
            .is_some()
    );
    fs::write(descriptor, descriptor_bytes).unwrap();
    assert!(path.exists());
    let restored = reopen(&first, "restored");
    let reply = load_product("war-casualties", &restored, "restored", &params, &context);
    assert_eq!(reply["data"]["unknownLabel"], "Mod casualty title");
    assert!(entry.get().is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_recorded_mod_is_verified_before_serving_a_cached_reply() {
    let root = root("changed-mod");
    let (first, context, path) = mod_source(&root);
    let params = json!({"branchId":"1", "tags":[], "top":null});
    let computed = load_product("war-casualties", &first, "first", &params, &context);
    assert_eq!(computed["data"]["unknownLabel"], "Mod casualty title");
    let entry = Entry::new(&first, "war-casualties", true, &params, &context).unwrap();
    let stored = entry.get().unwrap();
    fs::write(
        path,
        "\u{feff}l_english:\n CASUALTIES_UNKNOWN:0 \"Changed title\"\n",
    )
    .unwrap();
    let changed = reopen(&first, "changed");
    changed.name_countries(context.content.as_ref()).unwrap();
    assert!(
        changed
            .metadata()
            .unwrap()
            .warnings
            .iter()
            .any(|warning| warning.contains("hash mismatch"))
    );
    let live = load_product("war-casualties", &changed, "changed", &params, &context);
    assert_ne!(live["data"], computed["data"]);
    assert!(
        live["data"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("hash mismatch"))
    );
    assert_eq!(
        entry.get().unwrap(),
        stored,
        "a reply from changed content must not replace the recorded content's entry"
    );
    // The changed layer's installed hash keys its own entry.
    let changed_entry = Entry::new(&changed, "war-casualties", true, &params, &context).unwrap();
    assert_ne!(changed_entry.path, entry.path);
    // A served reply names its own content's changed layer once, however often it is decorated.
    let mut stack = vec!["kept".to_owned()];
    changed.append_changed_warnings(&mut stack, context.content.as_ref());
    changed.append_changed_warnings(&mut stack, context.content.as_ref());
    let named: Vec<_> = stack
        .iter()
        .filter(|line| line.contains("hash mismatch"))
        .collect();
    assert_eq!(named.len(), 1, "{stack:?}");
    assert_eq!(stack[0], "kept");
    // A reply admitted under other folders names its own folders' content, whatever is bound now.
    let mut other = context.content.clone().unwrap();
    other.options.cache_dir = root.join("other-cache");
    let mut own = vec!["kept".to_owned()];
    changed.append_changed_warnings(&mut own, Some(&other));
    assert_eq!(own, stack);
    changed.append_changed_warnings(&mut stack, context.content.as_ref());
    assert_eq!(own, stack);
    // A reply that reads no game content is kept without it.
    let recorded_only = load_product(
        "world-tension",
        &changed,
        "changed",
        &json!({"branchId":"1"}),
        &context,
    );
    assert!(
        !recorded_only["data"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("hash mismatch")),
        "{recorded_only}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_uncovered_hits_and_misses_prevent_reply_persistence_and_serving() {
    let root = root("explicit-uncovered");
    let (first, context) = source_with_content(vec![source(1, 100, Some(1))], &root);
    fs::create_dir_all(root.join("gfx/custom")).unwrap();
    fs::write(root.join("gfx/custom/icon.dds"), "synthetic").unwrap();
    let params = json!({"branchId":"1"});
    let entry = Entry::new(&first, "world-tension", false, &params, &context).unwrap();
    for path in [
        b"gfx/custom/icon.dds".as_slice(),
        b"gfx/custom/missing.dds".as_slice(),
    ] {
        let opened = reopen(&first, "uncovered");
        let content = opened
            .content(&context.content.as_ref().unwrap().options)
            .unwrap();
        let _ = content.layers.read_uncovered_path(path);
        let live = load(&opened, "live", &params, &context);
        assert!(
            live["data"]["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning
                    .as_str()
                    .unwrap()
                    .contains(std::str::from_utf8(path).unwrap()))
        );
        assert!(entry.get().is_none());
        entry.put(r#"{"stale":true}"#);
        let served = load(&opened, "live", &params, &context);
        assert_ne!(served["data"], json!({"stale":true}));
        fs::remove_file(&entry.path).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recording_fingerprint_covers_unsampled_bytes_in_later_blocks() {
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
    let root = root("corruption");
    let (first, context) = source_with_content(vec![source(1, 100, Some(1))], &root);
    let mut prefix = records(1, vec![source(1, 100, Some(1))]);
    let replay::spool::Record::SessionStart(session) = &mut prefix[0] else {
        unreachable!()
    };
    session.identity = first.replay_ref().identity.clone();
    let later = continuation_records(first.replay_ref(), 1, vec![scale_frame(1, 200, 5000)]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[prefix, later]).unwrap();
    assert_eq!(replay.index.len(), 2);
    let block = &replay.index[1];
    assert!(
        block.length > 8192,
        "fixture must exceed the former end samples"
    );
    let mut bytes = replay.bytes().to_vec();
    bytes[block.offset as usize + block.length as usize / 2] ^= 255;
    let damaged = from_bytes(bytes);
    assert!(
        damaged
            .replay_ref()
            .read_block(&damaged.replay_ref().index[1])
            .is_err()
    );
    let healthy = from_bytes(replay.bytes().to_vec());
    let params = json!({"branchId":"1"});
    let entry = Entry::new(&healthy, "world-tension", false, &params, &context).unwrap();
    entry.put(r#"{"stale":true}"#);
    let changed = Entry::new(&damaged, "world-tension", false, &params, &context).unwrap();
    assert_ne!(entry.path, changed.path);
    assert!(changed.get().is_none());
    assert_ne!(
        load(&damaged, "damaged", &params, &context)["data"],
        json!({"stale":true})
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn entries_include_effective_texture_decode_settings() {
    let root = root("texture-settings");
    let (first, context) = source_with_content(vec![source(1, 100, Some(1))], &root);
    let params = json!({"branchId":"1"});
    let entry = Entry::new(&first, "art", false, &params, &context).unwrap();
    entry.put(r#"{"width":3,"height":5}"#);
    fs::write(
        root.join("user/settings.txt"),
        "language=l_english force_pow2_textures=yes",
    )
    .unwrap();
    let padded = Entry::new(&reopen(&first, "padded"), "art", false, &params, &context).unwrap();
    assert_ne!(entry.path, padded.path);
    assert!(padded.get().is_none());
    fs::write(
        root.join("user/settings.txt"),
        "language=l_english force_pow2_textures=no force_pow2_textures=yes",
    )
    .unwrap();
    assert_eq!(
        padded.path,
        Entry::new(&first, "art", false, &params, &context)
            .unwrap()
            .path
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_compressed_or_expanding_entries_are_misses_and_replaced() {
    let root = root("bounded");
    let (first, context) = source_with_content(vec![source(1, 100, Some(1))], &root);
    let entry = Entry::new(
        &first,
        "world-tension",
        false,
        &json!({"branchId":"1"}),
        &context,
    )
    .unwrap();
    entry.put("{}");
    let mut prefix = MAGIC.to_vec();
    prefix.extend((entry.key.len() as u32).to_le_bytes());
    prefix.extend(entry.key.as_bytes());
    let expanded = vec![b' '; ENTRY_BUDGET + 1];
    let mut bomb = prefix.clone();
    zstd::stream::copy_encode(&expanded[..], &mut bomb, 3).unwrap();
    fs::write(&entry.path, bomb).unwrap();
    assert!(entry.get().is_none());
    entry.put("{}");
    assert_eq!(entry.get().as_deref(), Some("{}"));
    let mut oversized = prefix;
    oversized.extend(zstd::encode_all(&b"{}"[..], 3).unwrap());
    // A valid skippable frame adds compressed bytes without expanding output.
    oversized.extend(0x184d2a50_u32.to_le_bytes());
    oversized.extend((ENTRY_BUDGET as u32).to_le_bytes());
    oversized.resize(oversized.len() + ENTRY_BUDGET, 0);
    fs::write(&entry.path, oversized).unwrap();
    assert!(entry.get().is_none());
    entry.put("{}");
    assert_eq!(entry.get().as_deref(), Some("{}"));
    let mut legacy = b"HRC1".to_vec();
    legacy.extend((entry.key.len() as u32).to_le_bytes());
    legacy.extend(entry.key.as_bytes());
    legacy.extend(zstd::encode_all(&b"{}"[..], 3).unwrap());
    fs::write(&entry.path, legacy).unwrap();
    assert!(entry.get().is_none());
    entry.put("{}");
    assert_eq!(entry.get().as_deref(), Some("{}"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_later_source_of_the_same_recording_is_answered_from_disk() {
    let root = root("answer");
    let (first, context) = source_with_content(vec![source(1, 100, Some(1))], &root);
    let params = json!({"branchId": "1"});
    let computed = load(&first, "first", &params, &context);
    let entry = Entry::new(&first, "world-tension", false, &params, &context).unwrap();
    let stored: Value = serde_json::from_str(&entry.get().unwrap()).unwrap();
    assert_eq!(stored, computed["data"]);

    // The recording moved: a stored reply answers without loading.
    entry.put(r#"{"stored":true}"#);
    let later = reopen(&first, "moved.battleplan");
    let answered = load(&later, "later", &params, &context);
    assert_eq!(answered["sourceId"], "later");
    assert_eq!(answered["product"], "world-tension");
    assert_eq!(answered["data"], json!({"stored": true}));
    fs::remove_dir_all(root).unwrap();
}

/// A reply of recorded-only work is the recording's own whatever visual mods are selected: it is
/// kept under a selection, and served from the same entry once the selection changes. A reply
/// admitted on a binding the source has since left is not kept.
#[test]
fn recorded_only_replies_are_kept_whatever_the_selection() {
    let root = root("recorded_only");
    let (source, context) =
        source_with_content(vec![crate::test_fixtures::source(1, 100, Some(1))], &root);
    let folders = context.content.unwrap();
    let select = |name: &str| {
        source.bind(
            Some((&folders.options, folders.allow_unverified)),
            crate::presentation::Presentation {
                visual_mods: vec![installed_content_core::content::visual::Choice {
                    id: format!("mod/{name}.mod"),
                    name: name.into(),
                }],
                revision: name.into(),
            },
        );
        LoadContext {
            content: source.admit(crate::source::Work::Recorded),
        }
    };
    let params = json!({"branchId": "1"});
    let art = select("art");
    let computed = load(&source, "art", &params, &art);
    let entry = Entry::new(&source, "world-tension", false, &params, &art).unwrap();
    let stored: Value =
        serde_json::from_str(&entry.get().expect("kept under a selection")).unwrap();
    assert_eq!(stored, computed["data"]);
    // Another selection reads the same entry.
    let other = select("other");
    assert_eq!(
        Entry::new(&source, "world-tension", false, &params, &other)
            .unwrap()
            .path,
        entry.path
    );
    entry.put(r#"{"stored":true}"#);
    assert_eq!(
        load(&source, "other", &params, &other)["data"],
        json!({"stored": true})
    );
    // A reply admitted on the retired binding is drawn but not kept.
    fs::remove_file(&entry.path).unwrap();
    load(&source, "late", &params, &art);
    assert!(
        entry.get().is_none(),
        "a retired binding's reply is not kept"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn entries_are_kept_apart_by_request_recording_and_content() {
    let root = root("apart");
    let (first, mut context) = source_with_content(vec![source(1, 100, Some(1))], &root);
    let path = |source: &Source, params: Value, context: &LoadContext| {
        Entry::new(source, "world-tension", false, &params, context)
            .unwrap()
            .path
    };
    let base = path(&first, json!({"branchId": "1"}), &context);
    assert_eq!(
        base,
        path(&reopen(&first, "copy"), json!({"branchId": "1"}), &context)
    );
    assert_ne!(base, path(&first, json!({"branchId": "2"}), &context));
    let (other, _) = source_with_content(vec![source(1, 101, Some(1))], &root);
    assert_ne!(base, path(&other, json!({"branchId": "1"}), &context));
    let content = context.content.as_mut().unwrap();
    content.allow_unverified = !content.allow_unverified;
    assert_ne!(base, path(&first, json!({"branchId": "1"}), &context));
    context.content.as_mut().unwrap().allow_unverified ^= true;
    fs::write(root.join("user/settings.txt"), "language=l_french").unwrap();
    assert_ne!(base, path(&first, json!({"branchId": "1"}), &context));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pruning_keeps_the_most_recently_used_entries() {
    let root = root("prune");
    let folder = root.join("recording");
    fs::create_dir_all(&folder).unwrap();
    for (name, age) in [("old", 30), ("used", 20), ("new", 10)] {
        let path = folder.join(name);
        fs::write(&path, [0; 100]).unwrap();
        let used = SystemTime::now() - std::time::Duration::from_secs(age);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(used)
            .unwrap();
    }
    prune(&root, 300).unwrap();
    assert_eq!(size(&root), 300);
    prune(&root, 280).unwrap();
    assert!(!folder.join("old").exists());
    assert!(folder.join("used").exists() && folder.join("new").exists());
    clear(&root).unwrap();
    assert_eq!(size(&root), 0);
}

#[test]
fn file_backed_reference_cache_uses_recording_path_and_opened_metadata_revision() {
    let directory = std::env::temp_dir().join(format!("reply-cache-file-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![source(1, 100, Some(1))],
        )],
    )
    .unwrap();
    let left = directory.join("left.battleplan");
    fs::create_dir(directory.join("elsewhere")).unwrap();
    let right = directory.join("elsewhere").join("right.battleplan");
    fs::write(&left, replay.bytes()).unwrap();
    fs::write(&right, replay.bytes()).unwrap();
    let make = |path: PathBuf, revision: &str| {
        Source::from_replay(
            SourceKey {
                source_id: revision.into(),
                revision: revision.into(),
            },
            path.clone(),
            Arc::new(Replay::open_path(&path).unwrap()),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let one = make(left.clone(), "1");
    let two = make(right, "1");
    let changed = make(left, "2");
    assert!(one.replay_ref().is_file_backed());
    let reopened = reopen(&one, one.path().to_str().unwrap());
    assert!(reopened.replay_ref().is_file_backed());
    assert!(Arc::ptr_eq(&one.replay(), &reopened.replay()));
    assert_ne!(recording(&one), recording(&two));
    assert_ne!(recording(&one), recording(&changed));
    drop((one, two, changed, reopened));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_write_queued_before_its_folder_is_cleared_or_past_the_queue_is_not_made() {
    let root = root("queued-writes");
    let entry = |name: &str| Entry {
        path: root.join("recording").join(format!("{name}.zst")),
        key: name.into(),
    };
    let written = |name: &str| root.join("recording").join(format!("{name}.zst")).exists();
    // Written before the clear or dropped after it, the entry is gone either way.
    entry("cleared").put_later(2, || Some("{}".into()));
    clear(&root).unwrap();
    settle();
    assert!(!written("cleared"));
    entry("dropped").put_later(QUEUED_BYTES + 1, || Some("{}".into()));
    entry("kept").put_later(2, || Some("{}".into()));
    settle();
    assert!(!written("dropped"), "past what may be queued");
    assert!(written("kept"), "queued after the clear");
    clear(&root).unwrap();
}

fn artwork_entry(root: &Path) -> Entry {
    Entry {
        path: root.join("recording").join("artwork.zst"),
        key: "artwork".into(),
    }
}

#[test]
fn an_entry_carries_the_images_its_reply_names_to_a_later_process() {
    let root = root("artwork");
    let entry = artwork_entry(&root);
    let shown = crate::artwork::png_url(&installed_content_image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![10, 20, 30, 255],
    })
    .unwrap();
    let reply = json!({ "icons": [shown, shown] }).to_string();
    entry.put(&reply);
    // Another process's table, which never drew the image.
    let later = Table::new(crate::artwork::base());
    assert!(later.url_bytes(&shown).is_none());
    assert_eq!(entry.get_in(&later).as_deref(), Some(reply.as_str()));
    assert_eq!(later.url_bytes(&shown), crate::artwork::url_bytes(&shown));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_reply_naming_an_image_it_cannot_carry_is_not_written() {
    let root = root("artwork-missing");
    let entry = artwork_entry(&root);
    let missing = format!("{}{}.png", crate::artwork::base(), "7f".repeat(16));
    entry.put(&json!({ "icon": missing }).to_string());
    assert!(!entry.path.exists());
    assert!(entry.get().is_none());
    let _ = fs::remove_dir_all(root);
}
