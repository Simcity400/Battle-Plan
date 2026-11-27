use super::*;
use std::{cell::Cell, time::Duration};

#[test]
fn strip_identity_is_repeatable_and_every_input_distinguishes_the_key() {
    use dashboard_base::test_fixtures::{replay, source};
    use serde_json::json;

    let replay = replay(vec![source(1, 100, Some(100))]).into_timelines(&Default::default()).unwrap();
    let timelines = replay.timelines().unwrap();
    let mut changed_piece = timelines.clone();
    changed_piece.list[0].pieces[0].declared -= 1;
    let mut changed_link = timelines.clone();
    changed_link.links[0].candidates.push(2);
    let file = FileKey { path: b"recording.battleplan", stamp: "1", root: [2; 16], number: 3, length: 4 };
    let build = ("dashboard".into(), [5; 24]);
    let changed_build = ("dashboard".into(), [6; 24]);
    let changed_program = ("desktop".into(), build.1);
    let binding: catalogue::Binding = (("game".into(), "user".into(), false), "cache".into(), Default::default(), Ok(false));
    let mut changed_binding = binding.clone();
    changed_binding.3 = Ok(true);
    let content = ([7; 32], binding);
    let changed_binding = (content.0, changed_binding);
    let changed_stack = ([8; 32], content.1.clone());
    let params = json!({"product":"nuclear-arsenal","asks":[{"branchId":"1","hour":100}],"whole":true});
    let mut changed_params = params.clone();
    changed_params["whole"] = json!(false);
    let key = StripKey {
        file, version: VERSION, campaign: b"campaign", timelines: Some(timelines), build: &build,
        artwork: "artwork:", content: Some(&content), params: &params,
    };
    let (revision, group) = key.hashes().unwrap();
    assert_eq!(key.hashes().unwrap(), (revision, group));
    for changed_file in [
        FileKey { path: b"copy.battleplan", ..file },
        FileKey { stamp: "2", ..file },
        FileKey { root: [1; 16], ..file },
        FileKey { number: 4, ..file },
        FileKey { length: 5, ..file },
    ] {
        let (next_revision, next_group) = StripKey { file: changed_file, ..key }.hashes().unwrap();
        assert_ne!(next_revision, revision);
        assert_ne!(next_group, group);
    }
    for changed in [
        StripKey { version: VERSION + 1, ..key },
        StripKey { campaign: b"other", ..key },
        StripKey { timelines: Some(&changed_piece), ..key },
        StripKey { timelines: Some(&changed_link), ..key },
        StripKey { build: &changed_build, ..key },
        StripKey { build: &changed_program, ..key },
        StripKey { artwork: "other:", ..key },
        StripKey { content: None, ..key },
        StripKey { content: Some(&changed_binding), ..key },
        StripKey { content: Some(&changed_stack), ..key },
        StripKey { params: &changed_params, ..key },
    ] {
        let (same_revision, next_group) = changed.hashes().unwrap();
        assert_eq!(same_revision, revision);
        assert_ne!(next_group, group);
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};

        let non_unicode_cache = |surrogate| {
            let (stack, mut binding) = content.clone();
            binding.1 = OsString::from_wide(&[u16::from(b'c'), surrogate]).into();
            let content = (stack, binding);
            StripKey { content: Some(&content), ..key }.hashes().unwrap().1
        };
        assert_ne!(non_unicode_cache(0xd800), non_unicode_cache(0xd801));
    }
}

#[test]
fn eviction_keeps_the_more_recently_used_group() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&folder).unwrap();
    let older = folder.join("older.bpst");
    let recent = folder.join("recent.bpst");
    for path in [&older, &recent] {
        fs::write(path, []).unwrap();
        File::options().write(true).open(path).unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1)).unwrap();
    }
    touch(&recent, &|| Ok(())).unwrap();
    let mut archive = Archive {
        folder: folder.clone(),
        recording: Recording { path: PathBuf::new(), revision: String::new() },
        library: folder.clone(),
        entries: HashMap::from([(older.clone(), 8), (recent.clone(), PAGE_STRIP_BYTES - 8)]),
        bytes: PAGE_STRIP_BYTES,
    };
    archive.reserve(&folder.join("next.bpst"), 8, &|| Ok(())).unwrap();
    assert!(!older.exists());
    assert!(recent.is_file());
    assert_eq!(archive.bytes, PAGE_STRIP_BYTES - 8);
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn priority_before_rename_preserves_the_complete_file() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let place = marker(&folder);
    place.save_serial(b"complete", || Ok(())).unwrap();
    let checks = Cell::new(0);
    let result = place.save_serial(b"next", || {
        checks.set(checks.get() + 1);
        if checks.get() == 1 { Ok(()) } else { Err(io::ErrorKind::WouldBlock.into()) }
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(place.load(8).unwrap(), b"complete");
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_late_hit_cannot_replace_the_latest_revision() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let library = root.join("library");
    fs::create_dir_all(&library).unwrap();
    let configured = move || Ok(library.clone());
    let make = |revision: &str, action| Pending {
        recording: Recording { path: root.join("outside.battleplan"), revision: revision.into() },
        campaign: blake3::hash(b"campaign"),
        key: blake3::hash(revision.as_bytes()),
        action,
    };
    let first = make("first", Action::Save(b"BPST\x01\0\x01\0".to_vec()));
    let first_path = group(&root.join("external"), first.key);
    let second = make("second", Action::Save(b"BPST\x01\0\x01\0".to_vec()));
    let second_path = group(&root.join("external"), second.key);
    let mut disk = None;
    let ready = || Ok(());
    Disk::keep(&mut disk, &root, &configured, first, &ready).unwrap();
    Disk::keep(&mut disk, &root, &configured, second, &ready).unwrap();
    let _ = Disk::keep(&mut disk, &root, &configured, make("first", Action::Touch(first_path)), &ready);
    assert!(second_path.is_file());
    drop(disk);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_sweep_requires_a_readable_library_and_preserves_an_unreadable_recording() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let library = folder.join("library");
    let root = folder.join("cache");
    let remember = |name: &str| {
        let slot = root.join(blake3::hash(name.as_bytes()).to_hex().as_str());
        let recording = Recording { path: library.join(format!("{name}.battleplan")), revision: String::new() };
        marker(&slot).save_serial(&serde_json::to_vec(&recording).unwrap(), || Ok(())).unwrap();
        (slot, recording.path)
    };
    let (removed, _) = remember("removed");
    let (unreadable, damaged) = remember("unreadable");
    let save = || Pending {
        recording: Recording { path: folder.join("outside.battleplan"), revision: String::new() },
        campaign: blake3::hash(b"current"),
        key: blake3::hash(b"group"),
        action: Action::Save(b"BPST\x01\0\x01\0".to_vec()),
    };
    let external = group(&root.join("external"), save().key);
    let configured = {
        let library = library.clone();
        move || Ok(library.clone())
    };
    let ready = || Ok(());
    let mut disk = None;
    assert_eq!(Disk::keep(&mut disk, &root, &configured, save(), &ready).unwrap_err().kind(), io::ErrorKind::NotFound);
    assert!(disk.is_none());
    assert!(removed.is_dir() && unreadable.is_dir() && !external.exists());
    fs::create_dir(&library).unwrap();
    fs::write(damaged, b"damaged recording header").unwrap();
    Disk::keep(&mut disk, &root, &configured, save(), &ready).unwrap();
    assert!(unreadable.is_dir() && !removed.exists() && external.is_file());
    drop(disk);
    fs::remove_dir_all(folder).unwrap();
}
