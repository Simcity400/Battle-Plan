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
