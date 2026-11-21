use super::*;
use std::io::Cursor;

fn identity() -> Identity {
    Identity {
        schema: [1; 32],
        campaign: b"campaign".to_vec(),
        content: b"content".to_vec(),
        capture_mode: 2,
    }
}

fn data(at: u32) -> Extent {
    Extent {
        kind: Kind::Data,
        run: 0,
        stream: at,
        first: 0,
        hours: 0,
        offset: 100,
        stored: 10,
        raw: 24,
        digest: [at as u8; 16],
    }
}

fn family(id: u32) -> SectionKey {
    SectionKey::new(SectionKind::FamilyAnswers, id)
}

fn temporary(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("container-tests-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn a_root_names_each_section_once_in_order_with_its_version() {
    let directory = Extent {
        kind: Kind::RunDirectory,
        ..data(0)
    };
    let root = Root {
        identity: identity(),
        revision: 4,
        state: Some(SlotRef {
            sequence: 3,
            digest: [7; 16],
        }),
        saved: Saved::No(Some(Revision {
            number: 2,
            length: 4_096,
            root: [9; 16],
        })),
        sections: vec![
            Section {
                key: SectionKey::WORLD_LOG,
                version: schema::WORLD_LOG,
                frozen: Frozen::default(),
                open: vec![directory, Extent { run: 1, ..directory }],
            },
            Section {
                key: family(3),
                version: 2,
                frozen: Frozen::default(),
                open: vec![data(0)],
            },
        ],
        level: Some(-7),
    };
    assert_eq!(Root::read(&root.write().unwrap()).unwrap(), root);
    let unrecorded = Root { level: None, ..root.clone() };
    assert_eq!(Root::read(&unrecorded.write().unwrap()).unwrap(), unrecorded);
    assert!(root.write().unwrap().len() > unrecorded.write().unwrap().len());
    assert!(root.open(family(3), 2).unwrap().is_some());
    let older = root.open(family(3), 3).unwrap_err().to_string();
    assert!(older.contains("schema version 2, older"), "{older}");
    let mut swapped = root.clone();
    swapped.sections.swap(0, 1);
    assert!(swapped.write().is_err());
    let mut misplaced = root.clone();
    misplaced.sections[0].open.swap(0, 1);
    assert!(misplaced.write().is_err());
    let mut foreign = root;
    foreign.sections[1].open[0].kind = Kind::Controls;
    assert!(foreign.write().is_err());
}

fn entry_at(file: &[u8], key: SectionKey, frozen: &Frozen, at: u64) -> Extent {
    let (mut page, mut level, mut at) = match frozen.locate(at).unwrap() {
        Place::Extent(extent) => return *extent,
        Place::Page { page, level, at } => (*page, level, at),
    };
    loop {
        let raw = decompress(page.stored_in(file).unwrap(), Some(page.raw)).unwrap();
        let entries = read_page(&raw, &page, key).unwrap();
        let below = span(level - 1);
        let entry = entries[(at / below) as usize];
        if level == 1 {
            return entry;
        }
        (page, level, at) = (entry, level - 1, at % below);
    }
}

#[test]
fn a_section_pages_its_frozen_extents_a_fanout_at_a_time() {
    let mut writer = ContainerWriter::create(Cursor::new(Vec::new()), identity(), 3, None).unwrap();
    writer.section(family(1), 1).unwrap();
    let count = (FANOUT * FANOUT + 3 * FANOUT + 5) as u32;
    for at in 0..count {
        let tags = Tags {
            stream: at,
            ..Tags::default()
        };
        let extent = writer.put(Kind::Stored, tags, 1, &[at as u8]).unwrap();
        writer.append(family(1), extent).unwrap();
    }
    assert!(writer.append(family(1), data(0)).is_err());
    writer.commit(None, true, |_| Ok(())).unwrap();
    let section = writer.section_of(family(1)).unwrap().clone();
    assert_eq!(section.len(), u64::from(count));
    let levels: Vec<usize> = section.frozen.levels().iter().map(Vec::len).collect();
    assert_eq!(levels, vec![5, 3, 1]);
    let file = writer.file_mut().get_ref().clone();
    for at in [0_u64, 31, 32, 1023, 1024, 1024 + 95, u64::from(count) - 1] {
        assert_eq!(u64::from(entry_at(&file, family(1), &section.frozen, at).stream), at);
    }
    assert!(section.frozen.locate(u64::from(count)).is_none());
}

#[test]
fn a_crash_resumes_at_the_newest_root_whose_state_slot_verifies() {
    let directory = temporary("resume");
    let path = directory.join("campaign.battleplan");
    let state_path = StateFile::path_for(&path);
    let file = std::fs::File::create(&path).unwrap();
    let state = StateFile::open(&state_path).unwrap();
    let mut writer = ContainerWriter::create(file, identity(), 3, Some(state)).unwrap();
    writer.section(family(1), 1).unwrap();
    let entry = |bytes: &[u8]| StateEntry {
        owner: family(1),
        version: 1,
        bytes: bytes.to_vec(),
    };
    for (at, state) in [b"first".as_slice(), b"second", b"third"].into_iter().enumerate() {
        let extent = writer.put_zstd(Kind::Data, Tags::default(), state, 3).unwrap();
        writer.append(family(1), extent).unwrap();
        let revision = writer.commit(Some(&[entry(state)]), true, |file| file.sync_data()).unwrap();
        assert_eq!(revision.number, at as u64 + 1);
    }
    drop(writer);
    let read = |path: &std::path::Path| std::fs::read(path).unwrap();
    let recovered = recover(read(&path).as_slice(), Some(&StateFile::open(&state_path).unwrap())).unwrap();
    assert_eq!((recovered.root.revision, recovered.skipped), (3, 0));
    assert_eq!(recovered.state, vec![entry(b"third")]);
    let mut damaged = read(&state_path);
    damaged[16] ^= 1;
    let damaged_path = directory.join("damaged.state");
    std::fs::write(&damaged_path, &damaged).unwrap();
    let earlier = recover(read(&path).as_slice(), Some(&StateFile::open(&damaged_path).unwrap())).unwrap();
    assert_eq!((earlier.root.revision, earlier.skipped), (2, 1));
    assert_eq!(earlier.state, vec![entry(b"second")]);
    let named = recovered.root.state;
    let mut state = StateFile::open(&state_path).unwrap();
    state.write(4, b"unnamed", named).unwrap();
    drop(state);
    let recovered = recover(read(&path).as_slice(), Some(&StateFile::open(&state_path).unwrap())).unwrap();
    assert_eq!((recovered.root.revision, recovered.skipped), (3, 0));
    let mut damaged = read(&state_path);
    damaged[16] ^= 1;
    std::fs::write(&state_path, &damaged).unwrap();
    let held = StateFile::open(&state_path).unwrap();
    assert_eq!(held.slots().collect::<Vec<_>>(), vec![4]);
    assert!(recover(read(&path).as_slice(), Some(&held)).is_err());
    assert!(recover(read(&path).as_slice(), None).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn finishing_keeps_each_familys_state_once_and_removes_the_state_file() {
    let directory = temporary("finish");
    let path = directory.join("campaign.battleplan");
    let state_path = StateFile::path_for(&path);
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ContainerWriter::create(file, identity(), 3, Some(StateFile::open(&state_path).unwrap())).unwrap();
    writer.section(family(2), 5).unwrap();
    let state = StateEntry {
        owner: family(2),
        version: 5,
        bytes: b"family two".to_vec(),
    };
    writer.commit(Some(std::slice::from_ref(&state)), true, |file| file.sync_data()).unwrap();
    assert!(state_path.exists());
    writer.finish(&[state], true, |file| file.sync_data()).unwrap();
    assert!(!state_path.exists());
    let bytes = std::fs::read(&path).unwrap();
    let recovered = recover(bytes.as_slice(), None).unwrap();
    assert_eq!(recovered.root.state, None);
    let kept = recovered
        .root
        .open(SectionKey::new(SectionKind::FinalState, 2), 5)
        .unwrap()
        .unwrap();
    let extent = kept.open[0];
    assert_eq!(decompress(extent.stored_in(&bytes).unwrap(), Some(extent.raw)).unwrap(), b"family two");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_map_restore_section_names_each_points_hour_ordinal_and_spacing() {
    use restore_points::{MapRestoreWriter, Payload, Schedule, point_of};
    let mut writer = ContainerWriter::create(Cursor::new(Vec::new()), identity(), 3, None).unwrap();
    let schedule = Schedule { spacing_hours: 24 };
    assert!(schedule.due(100, 148) && !schedule.due(100, 147) && !schedule.due(100, 76));
    let mut points = MapRestoreWriter::new(&mut writer, 1, 3, schedule).unwrap();
    let first = points.add(&mut writer, 100, b"opaque page image", Payload::Zstd(3)).unwrap();
    let second = points.add(&mut writer, 124, b"opaque change", Payload::Stored).unwrap();
    assert_eq!((point_of(&first), point_of(&second)), ((100, 0, 24), (124, 1, 24)));
    writer.commit(None, true, |_| Ok(())).unwrap();
    assert_eq!(MapRestoreWriter::new(&mut writer, 1, 3, schedule).unwrap().points(), 2);
    assert!(MapRestoreWriter::new(&mut writer, 1, 4, schedule).is_err());
    let file = writer.file_mut().get_ref().clone();
    assert_eq!(second.stored_in(&file).unwrap(), b"opaque change");
}

#[test]
fn a_catalogue_gives_each_key_one_id_and_reads_its_chunks_in_order() {
    let mut catalogue = catalogue::Catalogue::new();
    assert_eq!(catalogue.insert(b"GER").unwrap(), 0);
    assert_eq!(catalogue.insert(b"ENG").unwrap(), 1);
    assert_eq!(catalogue.insert(b"GER").unwrap(), 0);
    let chunk = catalogue.pending().unwrap().unwrap();
    let mut read = catalogue::Catalogue::new();
    read.read_chunk(&chunk).unwrap();
    assert_eq!((read.id(b"ENG"), read.key(0)), (Some(1), Some(b"GER".as_slice())));
    assert!(read.read_chunk(&chunk).is_err());
}

#[test]
fn a_reader_opens_the_newest_saved_revision() {
    let mut writer = ContainerWriter::create(Cursor::new(Vec::new()), identity(), 3, None).unwrap();
    writer.section(family(1), 1).unwrap();
    let add = |writer: &mut ContainerWriter<Cursor<Vec<u8>>>, saved: bool| {
        let extent = writer.put(Kind::Stored, Tags::default(), 1, b"x").unwrap();
        writer.append(family(1), extent).unwrap();
        writer.commit(None, saved, |_| Ok(())).unwrap()
    };
    let opened = |file: &[u8], open: Open| {
        locate(file, open).unwrap().map(|(root, length)| {
            let read = read_root(file, &root).unwrap();
            (read.revision, length, read.section(family(1)).unwrap().len())
        })
    };
    let first = add(&mut writer, false);
    let file = writer.file_mut().get_ref().clone();
    assert_eq!(opened(&file, Open::Saved), None);
    assert_eq!(opened(&file, Open::Newest), Some((1, first.length, 1)));
    assert_eq!(writer.saved(), None);
    let saved = add(&mut writer, true);
    let second = add(&mut writer, false);
    let third = add(&mut writer, false);
    assert_eq!(writer.saved(), Some(saved));
    let mut file = writer.file_mut().get_ref().clone();
    assert_eq!(opened(&file, Open::Saved), Some((2, saved.length, 2)));
    assert_eq!(opened(&file, Open::Newest), Some((4, third.length, 4)));
    file.extend_from_slice(&[0x5a; 300]);
    file.extend_from_slice(FOOTER_MAGIC);
    assert_eq!(opened(&file, Open::Saved), Some((2, saved.length, 2)));
    assert_eq!(opened(&file, Open::Newest), Some((4, third.length, 4)));
    file.truncate(third.length as usize);
    let (root, length) = locate(file.as_slice(), Open::Newest).unwrap().unwrap();
    let at = Revision {
        number: 4,
        length,
        root: root.digest,
    };
    let resumed = read_root(file.as_slice(), &root).unwrap();
    assert_eq!(resumed.saved, Saved::No(Some(saved)));
    let mut writer = ContainerWriter::resume(Cursor::new(file.clone()), resumed, at, 3, None).unwrap();
    assert_eq!(writer.saved(), Some(saved));
    let marked = writer.commit(None, true, |_| Ok(())).unwrap();
    assert_eq!(writer.saved(), Some(marked));
    let after = writer.file_mut().get_ref().clone();
    assert_eq!(opened(&after, Open::Saved), Some((5, marked.length, 4)));
    assert!(marked.length - third.length < 400, "a save appends a root and a footer");
    file.truncate(saved.length as usize);
    assert_eq!(opened(&file, Open::Saved), Some((2, saved.length, 2)));
    assert_eq!(opened(&file, Open::Newest), Some((2, saved.length, 2)));
    let mut damaged = after[..second.length as usize].to_vec();
    damaged[saved.length as usize - FOOTER_BYTES + 20] ^= 1;
    assert!(locate(damaged.as_slice(), Open::Saved).is_err());
    assert_eq!(opened(&damaged, Open::Newest), Some((3, second.length, 3)));
}

#[test]
fn a_root_records_the_level_every_revision_of_its_file_was_written_at() {
    let newest = |file: &[u8]| {
        let (root, length) = locate(file, Open::Newest).unwrap().unwrap();
        let read = read_root(file, &root).unwrap();
        let at = Revision { number: read.revision, length, root: root.digest };
        (read, at)
    };
    let mut writer = ContainerWriter::create(Cursor::new(Vec::new()), identity(), 7, None).unwrap();
    writer.section(family(1), 1).unwrap();
    writer.commit(None, true, |_| Ok(())).unwrap();
    let file = writer.into_inner().into_inner();
    let (root, at) = newest(&file);
    assert_eq!(root.level, Some(7));
    for (level, recorded) in [(7, Some(7)), (19, None)] {
        let mut writer = ContainerWriter::resume(Cursor::new(file.clone()), root.clone(), at, level, None).unwrap();
        let extent = writer.put_zstd(Kind::Data, Tags::default(), b"more", level).unwrap();
        writer.append(family(1), extent).unwrap();
        writer.commit(None, true, |_| Ok(())).unwrap();
        assert_eq!(newest(&writer.into_inner().into_inner()).0.level, recorded, "taken up at {level}");
    }
    let mut writer = ContainerWriter::create_after(Cursor::new(Vec::new()), identity(), 19, root.revision, None).unwrap();
    writer.commit(None, true, |_| Ok(())).unwrap();
    let (replaced, _) = newest(&writer.into_inner().into_inner());
    assert_eq!((replaced.revision, replaced.level), (root.revision + 1, Some(19)));
}
