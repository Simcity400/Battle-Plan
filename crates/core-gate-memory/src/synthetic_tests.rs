use crate::{
    manifest::Selection,
    memory::{Census, Layout},
};
use replay::{
    columnar_file::{FileWriter, Hour, Settings},
    columnar_frames::{self, Limits, Recording},
};
use replay_columnar::{
    envelope,
    manifest::{Manifest, Status},
    walk::State,
};
use replay_data::rows::TableId;
use std::io::Cursor;

pub fn folder() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::path::Path::new("captures/core-gate-spools/tests");
    std::fs::create_dir_all(root).unwrap();
    loop {
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return path,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("{e}"),
        }
    }
}

#[test]
fn reference_counts_spill_and_apply_only_membership_deltas() {
    use crate::keyset::Keys;
    let cache = crate::keyset::Cache::reserved("synthetic references", 32 * 1024).unwrap();
    let path = folder().join("lookup");
    let mut keys = Keys::new(&path, false, &cache).unwrap();
    let high = u128::MAX - 40_000;
    keys.begin().unwrap();
    for at in (0..40_000).rev() {
        keys.adjust(high + at, true).unwrap();
        keys.adjust(high + at, true).unwrap();
    }
    assert!(!keys.contained(high).unwrap());
    assert!(keys.contains(high).unwrap());
    keys.begin().unwrap();
    keys.adjust(high, false).unwrap();
    assert!(keys.contains(high).unwrap());
    assert_eq!(keys.next_change(0).unwrap(), None);
    keys.adjust(high, false).unwrap();
    keys.adjust(7, true).unwrap();
    assert_eq!(keys.next_change(0).unwrap(), Some((7, false, true)));
    assert_eq!(keys.next_change(8).unwrap(), Some((high, true, false)));
    keys.begin().unwrap();
    assert!(!keys.contained(high).unwrap());
    let mut retained = Keys::new(&folder().join("retained"), true, &cache).unwrap();
    retained.begin().unwrap();
    retained.adjust(u128::MAX, true).unwrap();
    retained.begin().unwrap();
    retained.adjust(u128::MAX, false).unwrap();
    assert!(retained.contains(u128::MAX).unwrap());
}

#[test]
fn every_audited_lookup_needs_its_spooled_fields_and_reference() {
    use crate::{manifest::Manifest as GateManifest, readset::Mode};
    let text = include_str!("../manifest.toml");
    let original: GateManifest = toml::from_str(text).unwrap();
    assert!(original.clone().resolve().is_ok());
    for proof in crate::proof::reads().filter(|p| p.access != "upstream") {
        let mut manifest = original.clone();
        let family = manifest
            .families
            .iter_mut()
            .find(|f| f.name == proof.family)
            .unwrap();
        let entry = family
            .readset
            .iter_mut()
            .find(|t| t.name == proof.table)
            .unwrap();
        entry.spool = Mode::None;
        assert!(
            manifest.resolve().is_err(),
            "{} {}",
            proof.family,
            proof.table
        );
    }
    let mut missing = original.clone();
    missing
        .families
        .iter_mut()
        .find(|f| f.name == "construction")
        .unwrap()
        .references
        .clear();
    assert!(missing.resolve().is_err());
    for (family, table) in [
        ("equipment_losses", "SliceReads"),
        ("event_feed", "RelationTuples"),
    ] {
        let mut wider = original.clone();
        wider
            .families
            .iter_mut()
            .find(|f| f.name == family)
            .unwrap()
            .readset
            .iter_mut()
            .find(|t| t.name == table)
            .unwrap()
            .filter = crate::readset::Rows::All;
        assert!(wider.resolve().is_err());
    }
    let mut wider = original.clone();
    wider
        .families
        .iter_mut()
        .find(|f| f.name == "event_feed")
        .unwrap()
        .readset
        .iter_mut()
        .find(|t| t.name == "PoliticsCaptureCoverage")
        .unwrap()
        .hours = Some(crate::readset::Hours::Every);
    assert!(wider.resolve().is_err());
    for rule in crate::proof::READ_RULES {
        let mut missing = original.clone();
        let family = missing
            .families
            .iter_mut()
            .find(|f| f.name == rule.family)
            .unwrap();
        family
            .readset
            .iter_mut()
            .find(|t| t.name == rule.table)
            .unwrap()
            .read_rule = None;
        assert!(
            missing.resolve().is_err(),
            "{} {} read evidence",
            rule.family,
            rule.table
        );
    }
    let mut missing = original;
    let family = missing
        .families
        .iter_mut()
        .find(|f| f.name == "production")
        .unwrap();
    family
        .readset
        .iter_mut()
        .find(|t| t.name == "ProductionLineOutput")
        .unwrap()
        .fields = crate::manifest::Fields::Named(vec![]);
    assert!(missing.resolve().is_err());
}

#[test]
fn two_hand_built_hours_walk_without_inventing_missing_rows() {
    let schema = columnar_frames::schema().unwrap();
    let columns: Vec<_> = schema.iter().map(Vec::len).collect();
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema),
        campaign: b"synthetic".to_vec(),
        content: Vec::new(),
        capture_mode: 0,
    };
    let mut writer = FileWriter::create(
        Cursor::new(Vec::new()),
        identity,
        columns.clone(),
        Settings::RECORDING,
    )
    .unwrap();
    writer.start_run(1, 0, 10).unwrap();
    for hour in [10, 11] {
        let manifest = Manifest {
            branch: 1,
            hour,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: Vec::new(),
            statuses: vec![Status::NotRecorded; TableId::ALL.len()],
            read: vec![0; TableId::ALL.len().div_ceil(64)],
            samples: Vec::new(),
            sections: Vec::new(),
        };
        let encoded = manifest.write(&columns).unwrap();
        writer
            .hour(Hour {
                hour,
                manifest: &encoded,
                sections: &[],
                statuses: &manifest.statuses,
                read: &manifest.read,
            })
            .unwrap();
    }
    let bytes = writer.finish(|_| Ok(())).unwrap().file.into_inner();
    let mut recording = Recording::open(bytes, Limits::default()).unwrap();
    recording.select(&[TableId::Countries]);
    let mut census = Census::new(Layout::new(&[TableId::Countries], 0), &schema);
    let selection = Selection {
        table: TableId::Countries,
        fields: vec![0, 1],
        recorded_only: Default::default(),
        undeclared: false,
        subscription: "stream".into(),
        previous: false,
        read_rule: None,
    };
    let mut state = State::new(schema).unwrap();
    let mut seen = Vec::new();
    recording
        .walk_segment(
            &recording.segments()[0],
            &mut state,
            |state, manifest, _| {
                census.observe(state, false).unwrap();
                assert_eq!(
                    replay::columns::status(state, TableId::Countries),
                    Status::NotRecorded
                );
                assert_eq!(
                    census
                        .footprint(std::slice::from_ref(&selection))
                        .live_slots,
                    0
                );
                seen.push(manifest.hour);
                Ok(true)
            },
        )
        .unwrap();
    assert_eq!(seen, [10, 11]);
}
