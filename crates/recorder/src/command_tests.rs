use crate::test_support::replay;
use replay_capture_spool::spool::Record;
use replay_data::rows::TableId;
use replay_frame::frame::TableUpdate;
use std::{fs, path::Path};

#[test]
fn exact_metrics_cli_routes_both_forms_and_refuses_recorded_failure() {
    let directory = crate::test_support::allocation_test_directory("exact-metrics");
    let path = directory.join("synthetic.battleplan");
    fs::write(
        &path,
        replay(crate::test_support::allocation_records()).bytes(),
    )
    .unwrap();
    let args = vec![path.into_os_string(), "9".into(), "2".into(), "4".into()];
    crate::recording::command(args.clone()).unwrap();
    let mut explicit = vec!["metrics".into()];
    explicit.extend(args.clone());
    crate::recording::command(explicit).unwrap();
    let mut failed = crate::test_support::allocation_records();
    for record in &mut failed {
        if let Record::Frame(frame) = record {
            frame.tables[TableId::Countries.index()] = TableUpdate::Unavailable {
                reason: "synthetic acquisition failure".into(),
            };
        }
    }
    fs::write(Path::new(&args[0]), replay(failed).bytes()).unwrap();
    assert!(
        crate::recording::command(args)
            .unwrap_err()
            .contains("recorded failures")
    );
}

#[test]
fn sizes_route_through_the_cli() {
    let directory = crate::test_support::allocation_test_directory("sizes");
    let path = directory.join("synthetic.battleplan");
    fs::write(
        &path,
        crate::test_support::replay(crate::test_support::allocation_records()).bytes(),
    )
    .unwrap();
    crate::recording::command(vec![
        "sizes".into(),
        path.clone().into_os_string(),
        "--json".into(),
    ])
    .unwrap();
    assert!(crate::recording::command(vec!["sizes".into()]).is_err());
}

#[test]
fn recompress_routes_through_the_cli() {
    let directory = std::env::temp_dir().join(format!("recorder-recompress-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("synthetic.battleplan");
    fs::write(&path, replay_journal::fixtures::restored(recorder_derive::writers(), 7, &[(1, 100, 1)])).unwrap();
    let run = |level: &[&str]| {
        let mut args = vec!["recompress".into(), path.clone().into_os_string()];
        args.extend(level.iter().map(Into::into));
        crate::recording::command(args)
    };
    run(&["--level", "19"]).unwrap();
    assert_eq!(replay_files::recompress::level_of(&path).unwrap(), Some(19));
    run(&["--level=19"]).unwrap();
    assert!(run(&["--level", "high"]).is_err());
    assert!(run(&[]).is_err());
    fs::remove_dir_all(&directory).unwrap();
}
