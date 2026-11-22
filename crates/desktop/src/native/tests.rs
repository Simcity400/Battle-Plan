use super::pending::MAX_QUEUED_ACTIONS;
use dashboard::protocol::NativeCommand;
use super::*;

#[test]
fn resized_commands_coalesce_and_close_takes_priority() {
    let mut pending = Pending {
        visibility: Some(true),
        open: Some(("x".into(), Some("v1".into()))),
        close: true,
        ..Default::default()
    };
    assert!(matches!(
        pending.take_commands(false).as_slice(),
        [NativeCommand::Close]
    ));
    assert!(!pending.can_send(true));
}

#[test]
fn unload_forgets_only_its_recording_and_cannot_restore_it_after_restart() {
    let mut inner = Inner {
        requested_path: Some("A".into()),
        requested_revision: Some("v1".into()),
        status: HostStatus {
            viewer: ViewerState {
                path: Some("A".into()),
                revision: Some("v1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    inner.restore_commands();
    inner.forget_recording("B");
    assert!(inner.pending.open.is_some());
    inner.forget_recording("A");
    inner.restore_commands();
    assert!(inner.pending.open.is_none());
    assert!(inner.status.viewer.path.is_none());
}

#[test]
fn restart_restores_unacknowledged_open_and_seek() {
    let requested = SeekTarget {
        path: "new.battleplan".into(),
        revision: "new-version".into(),
        branch: "9".into(),
        hour: 456,
        playback: Some(dashboard::protocol::MapPlayback {
            playing: true,
            seek_sequence: 7,
            fraction: 0.375,
            hours_per_second: 1440.0,
            composed_at: 0.0,
            clock_id: "page-1".into(),
        }),
    };
    let mut inner = Inner {
        requested_path: Some("new.battleplan".into()),
        requested_revision: Some("new-version".into()),
        requested_seek: Some(requested.clone()),
        status: HostStatus {
            viewer: ViewerState {
                path: Some("old.battleplan".into()),
                branch: Some("2".into()),
                hour: Some(123),
                revision: Some("old-version".into()),
            },
            ..Default::default()
        },
        ..Default::default()
    };
    inner.restore_commands();
    assert_eq!(
        inner.pending.open,
        Some(("new.battleplan".into(), Some("new-version".into())))
    );
    assert_eq!(inner.pending.seek, Some(requested));
    inner.requested_path = None;
    inner.requested_revision = None;
    inner.requested_seek = None;
    inner.restore_commands();
    assert_eq!(
        inner.pending.open,
        Some(("old.battleplan".into(), Some("old-version".into())))
    );
    assert_eq!(
        inner.pending.seek,
        Some(SeekTarget {
            path: "old.battleplan".into(),
            revision: "old-version".into(),
            branch: "2".into(),
            hour: 123,
            playback: None,
        })
    );
}

#[test]
fn seek_acknowledgement_requires_the_same_recording_and_revision() {
    let target = SeekTarget {
        path: "first.battleplan".into(),
        revision: "v1".into(),
        branch: "3".into(),
        hour: 42,
        playback: None,
    };
    let mut state = ViewerState {
        path: Some("second.battleplan".into()),
        revision: Some("v1".into()),
        branch: Some("3".into()),
        hour: Some(42),
    };
    assert!(!target.matches(&state));
    state.path = Some(target.path.clone());
    state.revision = Some("v2".into());
    assert!(!target.matches(&state));
    state.revision = Some(target.revision.clone());
    assert!(target.matches(&state));
}

#[test]
fn hidden_map_defers_document_work_without_spinning() {
    let mut pending = Pending {
        open: Some(("next.battleplan".into(), Some("v2".into()))),
        seek: Some(SeekTarget {
            path: "next.battleplan".into(),
            revision: "v2".into(),
            branch: "3".into(),
            hour: 42,
            playback: None,
        }),
        visibility: Some(false),
        ..Default::default()
    };
    assert!(matches!(
        pending.take_commands(false).as_slice(),
        [NativeCommand::Visibility { visible: false }]
    ));
    assert!(!pending.can_send(false));
    assert!(pending.can_send(true));
    assert!(matches!(
        pending.take_commands(true).as_slice(),
        [NativeCommand::Open { .. }, NativeCommand::Seek { .. }]
    ));
}

use dashboard::transport::{MAP_CONTROLS_BUSY, transient_error};

#[test]
fn busy_map_controls_refuse_with_a_retryable_error() {
    let mut pending = Pending::default();
    for _ in 0..MAX_QUEUED_ACTIONS - 1 {
        pending
            .actions
            .push_back(dashboard::protocol::MapAction::Inspect);
    }
    assert!(pending.action_room().is_ok());
    pending
        .actions
        .push_back(dashboard::protocol::MapAction::Inspect);
    let refusal = pending.action_room().unwrap_err();
    assert!(refusal.starts_with(MAP_CONTROLS_BUSY));
    assert!(transient_error(&refusal));
}

#[test]
fn folders_applied_before_a_viewer_starts_are_kept_for_its_launch() {
    let host = NativeHost::default();
    let config = dashboard::Config {
        game_dir: r"C:\Game".into(),
        user_dir: r"C:\User".into(),
        older_game_dirs: Vec::new(),
    };
    host.set_folders(&config);
    let inner = host.shared.lock().unwrap();
    assert_eq!(inner.folders.as_ref(), Some(&config));
    assert!(inner.pending.actions.is_empty());
}

#[test]
fn display_choices_sent_while_no_viewer_runs_are_kept_for_one_starting() {
    use dashboard::protocol::MapAction;
    let host = NativeHost::popout(1, Arc::new(|| ()));
    host.set_display(MapAction::Vsync { enabled: true });
    host.set_display(MapAction::Vsync { enabled: false });
    host.set_display(MapAction::UiScale { scale: Some(1.5) });
    host.set_display(MapAction::FrameLimit { hz: Some(120.0) });
    host.set_display(MapAction::FrameLimit { hz: Some(60.0) });
    let inner = host.shared.lock().unwrap();
    assert!(inner.pending.actions.is_empty());
    assert_eq!(
        format!("{:?}", inner.display),
        format!(
            "{:?}",
            [
                (2, MapAction::Vsync { enabled: false }),
                (3, MapAction::UiScale { scale: Some(1.5) }),
                (5, MapAction::FrameLimit { hz: Some(60.0) })
            ]
        )
    );
    assert!(inner.pending.campaign_map);
    assert!(
        inner
            .popout
            .as_ref()
            .is_some_and(|window| window.parent == 1)
    );
    assert!(
        NativeHost::default()
            .shared
            .lock()
            .unwrap()
            .popout
            .is_none()
    );
}

#[test]
fn moving_the_view_never_starts_a_viewer() {
    let host = NativeHost::popout(1, Arc::new(|| ()));
    let bounds = ViewBounds {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    };
    host.set_bounds(bounds).unwrap();
    let inner = host.shared.lock().unwrap();
    assert!(inner.map.is_none());
    assert!(!inner.status.running);
    assert_eq!(inner.bounds, Some(bounds));
    assert_eq!(inner.pending.bounds, None);
}

#[test]
fn only_a_display_choice_sent_while_a_viewer_starts_replaces_its_launch_read() {
    use dashboard::protocol::MapAction;
    let host = NativeHost::default();
    let debug = |actions: &[MapAction]| format!("{actions:?}");
    let read = vec![
        MapAction::Vsync { enabled: true },
        MapAction::UiScale { scale: None },
        MapAction::FrameLimit { hz: None },
    ];
    host.set_display(MapAction::Vsync { enabled: false });
    host.set_display(MapAction::UiScale { scale: Some(1.5) });
    host.set_display(MapAction::FrameLimit { hz: Some(60.0) });
    let since = host.shared.lock().unwrap().display_sent;
    let launch = || {
        let inner = host.shared.lock().unwrap();
        debug(&launch_display(read.clone(), &inner.display, since))
    };
    assert_eq!(launch(), debug(&read));
    host.set_display(MapAction::Vsync { enabled: false });
    host.set_display(MapAction::FrameLimit { hz: Some(120.0) });
    assert_eq!(
        launch(),
        debug(&[
            MapAction::UiScale { scale: None },
            MapAction::Vsync { enabled: false },
            MapAction::FrameLimit { hz: Some(120.0) }
        ])
    );
}

#[test]
fn a_host_names_the_recording_it_shows_requests_and_seeks() {
    let host = NativeHost::popout(1, Arc::new(|| ()));
    assert!(host.recordings().is_empty());
    {
        let mut inner = host.shared.lock().unwrap();
        inner.status.viewer.path = Some("shown".into());
        inner.requested_path = Some("opening".into());
        inner.requested_seek = Some(SeekTarget {
            path: "seeking".into(),
            revision: "1".into(),
            branch: "1".into(),
            hour: 0,
            playback: None,
        });
    }
    assert_eq!(host.recordings(), ["shown", "opening", "seeking"]);
}

#[test]
fn a_waiting_controller_message_is_sent_even_while_hidden() {
    use dashboard::protocol::GamepadMessage;
    let mut pending = Pending::default();
    assert!(!pending.can_send(true));
    pending.gamepad = Some(GamepadMessage::Release);
    assert!(pending.can_send(false));
    let commands = pending.take_commands(false);
    assert!(commands.iter().any(|command| matches!(
        command,
        NativeCommand::Gamepad {
            message: GamepadMessage::Release
        }
    )));
    assert!(!pending.can_send(true));
}

#[test]
fn only_the_latest_camera_mode_waits_for_the_viewer() {
    use dashboard::protocol::{CameraMode, MapAction};
    let host = NativeHost::default();
    host.shared.lock().unwrap().status.ready = true;
    for (mode, overlays) in [
        (CameraMode::Fly, false),
        (CameraMode::Walk, true),
        (CameraMode::Map, false),
        (CameraMode::Fly, false),
        (CameraMode::Fly, true),
    ] {
        host.map_action(MapAction::CameraMode { mode, overlays })
            .unwrap();
    }
    host.map_action(MapAction::Inspect).unwrap();
    let inner = host.shared.lock().unwrap();
    let queued: Vec<_> = inner
        .pending
        .actions
        .iter()
        .map(|action| match action {
            MapAction::CameraMode { mode, overlays } => Some((*mode, *overlays)),
            _ => None,
        })
        .collect();
    assert_eq!(queued, [Some((CameraMode::Fly, true)), None]);
}

#[test]
fn the_status_carries_the_camera_mode_for_the_interface() {
    let status = HostStatus {
        camera_mode: dashboard::protocol::CameraMode::Walk,
        camera_overlays: true,
        ..Default::default()
    };
    let value = serde_json::to_value(&status).unwrap();
    assert_eq!(value["cameraMode"], "walk");
    assert_eq!(value["cameraOverlays"], true);
    let fresh = serde_json::to_value(HostStatus::default()).unwrap();
    assert_eq!(
        (&fresh["cameraMode"], &fresh["cameraOverlays"]),
        (&"map".into(), &false.into())
    );
}

#[test]
fn only_the_latest_aircraft_choice_waits_for_the_viewer() {
    use dashboard::protocol::{CameraMode, MapAction};
    let host = NativeHost::default();
    host.shared.lock().unwrap().status.ready = true;
    host.map_action(MapAction::CameraMode {
        mode: CameraMode::Fly,
        overlays: false,
    })
    .unwrap();
    for index in 0..40 {
        host.map_action(MapAction::FlyAircraft {
            aircraft: Some(format!("plane_{index}_entity")),
        })
        .unwrap();
    }
    host.map_action(MapAction::FlyAircraft { aircraft: None })
        .unwrap();
    let inner = host.shared.lock().unwrap();
    let queued: Vec<_> = inner.pending.actions.iter().cloned().collect();
    assert!(matches!(
        queued.as_slice(),
        [
            MapAction::CameraMode {
                mode: CameraMode::Fly,
                overlays: false
            },
            MapAction::FlyAircraft { aircraft: None }
        ]
    ));
}
