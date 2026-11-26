use super::pending::MAX_QUEUED_ACTIONS;
use dashboard_core::protocol::NativeCommand;
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
fn popover_holes_wait_only_for_a_running_map_and_follow_a_restart() {
    let hole = ViewBounds { x: 10, y: 20, width: 300, height: 200 };
    let mut inner = Inner { status: HostStatus { ready: true, visible: true, ..Default::default() }, ..Default::default() };
    assert!(inner.set_holes(vec![hole]).unwrap().is_none(), "no running map has nothing to wait for");
    assert!(inner.pending.hole_waiters.is_empty());
    assert!(inner.set_holes(vec![ViewBounds { width: 0, ..hole }]).is_err());
    inner.pending.holes = None;
    inner.restore_commands();
    assert_eq!(inner.pending.holes, Some(vec![hole]), "a restarted map gets the open popovers' holes");
    inner.set_holes(Vec::new()).unwrap();
    inner.pending.holes = None;
    inner.restore_commands();
    assert_eq!(inner.pending.holes, None);
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
        playback: Some(dashboard_core::protocol::MapPlayback {
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

use dashboard_core::transport::{MAP_CONTROLS_BUSY, transient_error};

#[test]
fn busy_map_controls_refuse_with_a_retryable_error() {
    let mut pending = Pending::default();
    for _ in 0..MAX_QUEUED_ACTIONS - 1 {
        pending
            .actions
            .push_back(dashboard_core::protocol::MapAction::Inspect);
    }
    assert!(pending.action_room().is_ok());
    pending
        .actions
        .push_back(dashboard_core::protocol::MapAction::Inspect);
    let refusal = pending.action_room().unwrap_err();
    assert!(refusal.starts_with(MAP_CONTROLS_BUSY));
    assert!(transient_error(&refusal));
}

#[test]
fn folders_applied_before_a_viewer_starts_are_kept_for_its_launch() {
    let host = NativeHost::default();
    let config = dashboard_core::config::Config {
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
    use dashboard_core::protocol::MapAction;
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
    use dashboard_core::protocol::MapAction;
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
fn a_waiting_controller_message_is_sent_even_while_hidden() {
    use dashboard_core::protocol::GamepadMessage;
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
    use dashboard_core::protocol::{CameraMode, MapAction};
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
fn only_the_latest_aircraft_choice_waits_for_the_viewer() {
    use dashboard_core::protocol::{CameraMode, MapAction};
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

#[test]
fn retired_clock_origin_seeks_cannot_overwrite_pending_seek_or_reopen_previous_recording() {
    let playback = |clock_id: &str, seek_sequence: u64| {
        Some(dashboard_core::protocol::MapPlayback {
            playing: true,
            seek_sequence,
            fraction: 0.0,
            hours_per_second: 1440.0,
            composed_at: 100.0,
            clock_id: clock_id.into(),
        })
    };
    let host = NativeHost::default();
    host.seek(
        "old.battleplan".into(),
        "rev-old".into(),
        "1".into(),
        100,
        playback("page-A", 1),
    )
    .unwrap();
    host.seek(
        "new.battleplan".into(),
        "rev-new".into(),
        "2".into(),
        200,
        playback("page-B", 1),
    )
    .unwrap();
    host.seek(
        "old.battleplan".into(),
        "rev-old".into(),
        "1".into(),
        105,
        playback("page-A", 2),
    )
    .unwrap();
    host.seek(
        "new.battleplan".into(),
        "rev-new".into(),
        "2".into(),
        999,
        playback("page-A", 3),
    )
    .unwrap();
    let inner = host.shared.lock().unwrap();
    assert_eq!(inner.requested_path.as_deref(), Some("new.battleplan"));
    assert_eq!(inner.requested_revision.as_deref(), Some("rev-new"));
    assert_eq!(
        inner.pending.open,
        Some(("new.battleplan".into(), Some("rev-new".into())))
    );
    let pending_seek = inner.pending.seek.as_ref().expect("B seek stays queued");
    assert_eq!(pending_seek.path, "new.battleplan");
    assert_eq!(pending_seek.hour, 200);
    assert_eq!(
        pending_seek.playback.as_ref().map(|clock| clock.clock_id.as_str()),
        Some("page-B")
    );
    let requested_seek = inner.requested_seek.as_ref().expect("B seek stays requested");
    assert_eq!(requested_seek.path, "new.battleplan");
    assert_eq!(requested_seek.hour, 200);
    drop(inner);

    let popout = NativeHost::popout(0, Arc::new(|| {}));
    popout
        .seek(
            "popout.battleplan".into(),
            "rev-pop".into(),
            "1".into(),
            50,
            playback("page-A", 1),
        )
        .unwrap();
    let popout_inner = popout.shared.lock().unwrap();
    assert_eq!(
        popout_inner.pending.seek.as_ref().map(|seek| seek.hour),
        Some(50),
        "origin retirement is scoped per NativeHost"
    );
}

#[test]
fn dashboard_snapshot_envelope_gates_empty_clears_and_filters_state_replies_by_origin_before_sequence() {
    use dashboard_core::protocol::{DashboardMapState, DashboardMapView, ViewBounds};

    let bounds = ViewBounds {
        x: 0,
        y: 0,
        width: 100,
        height: 100,
    };
    let make_view = |id: &str, clock_id: &str, sequence: u64, hour: i32, playing: bool| {
        DashboardMapView {
            options: Default::default(),
            id: id.into(),
            source_id: "source".into(),
            path: "campaign.battleplan".into(),
            revision: "rev-1".into(),
            branch_id: "1".into(),
            playing,
            hours_per_second: 24.0,
            hour,
            fraction: 0.0,
            sequence,
            seek_sequence: Some(sequence),
            composed_at: 1000.0,
            clock_id: clock_id.into(),
            bounds,
            clip: bounds,
            visible: true,
            focus_province: None,
            battle: None,
        }
    };
    let make_state = |id: &str, clock_id: &str, sequence: u64, hour: i32| DashboardMapState {
        id: id.into(),
        clock_id: clock_id.into(),
        source_id: Some("source".into()),
        revision: Some("rev-1".into()),
        branch_id: Some("1".into()),
        hour: Some(hour),
        fraction: Some(0.0),
        sequence,
        requested_sequence: sequence,
        loading: false,
        ..Default::default()
    };

    let mut inner = Inner {
        active_dashboard: true,
        ..Default::default()
    };
    let view_a = make_view("map-1", "page-A", 80, 100, true);
    let view_b = make_view("map-1", "page-B", 0, 260, false);

    assert!(inner.apply_dashboard_views("page-A", std::slice::from_ref(&view_a)).unwrap());
    assert!(inner.apply_dashboard_views("page-B", &[view_b]).unwrap());
    assert!(!inner.apply_dashboard_views("page-A", &[]).unwrap());
    assert!(inner.status.visible, "late empty clear from A must not hide B's widgets");
    let kept = inner.dashboard_views.as_ref().expect("B's snapshot survives");
    assert_eq!(kept.clock_id.as_deref(), Some("page-B"));
    assert_eq!(kept.views.len(), 1);
    assert_eq!(kept.views[0].hour, 260);
    assert_eq!(
        inner.pending.maps.as_ref().map(|snap| (snap.clock_id.as_deref(), snap.views.len())),
        Some((Some("page-B"), 1))
    );

    let reply_a = make_state("map-1", "page-A", 80, 100);
    let reply_b = make_state("map-1", "page-B", 0, 260);
    assert!(
        inner.filter_map_states(vec![reply_a.clone()]).is_empty(),
        "A/80 arriving before B/0 must be filtered out before sequence comparison"
    );
    let kept_b = inner.filter_map_states(vec![reply_b]);
    assert_eq!(kept_b.len(), 1);
    assert_eq!(kept_b[0].clock_id, "page-B");
    assert_eq!(kept_b[0].hour, Some(260));
    assert!(!kept_b[0].loading);
    assert!(
        inner.filter_map_states(vec![reply_a]).is_empty(),
        "A/80 arriving after B/0 must be filtered out before sequence comparison"
    );

    assert!(inner.apply_dashboard_views("page-B", &[]).unwrap());
    assert!(!inner.status.visible);
    assert_eq!(
        inner.dashboard_views.as_ref().map(|snap| (snap.clock_id.as_deref(), snap.views.len())),
        Some((Some("page-B"), 0))
    );
    assert!(!inner.apply_dashboard_views("page-A", &[view_a]).unwrap());
    assert_eq!(
        inner.dashboard_views.as_ref().map(|snap| (snap.clock_id.as_deref(), snap.views.len())),
        Some((Some("page-B"), 0)),
        "late populated snapshot from A after B's legitimate empty clear must be ignored"
    );
    assert_eq!(
        inner.pending.maps.as_ref().map(|snap| (snap.clock_id.as_deref(), snap.views.len())),
        Some((Some("page-B"), 0))
    );
}

#[test]
fn select_view_and_restore_after_stamped_campaign_seek_stamp_empty_clear_with_active_origin() {
    let host = NativeHost::default();
    host.seek(
        "campaign.battleplan".into(),
        "rev-1".into(),
        "1".into(),
        100,
        Some(dashboard_core::protocol::MapPlayback {
            playing: false,
            seek_sequence: 1,
            fraction: 0.0,
            hours_per_second: 24.0,
            composed_at: 100.0,
            clock_id: "page-A".into(),
        }),
    )
    .unwrap();
    assert!(host.shared.lock().unwrap().dashboard_views.is_none());

    host.select_view(true).unwrap();
    let commands = host.shared.lock().unwrap().pending.take_commands(true);
    assert_eq!(
        commands,
        vec![NativeCommand::MapViews {
            clock_id: Some("page-A".into()),
            views: Vec::new(),
        }],
        "switching from the campaign map to Settings with no cached dashboard views must stamp the empty clear with the active origin"
    );

    let mut inner = host.shared.lock().unwrap();
    inner.restore_commands();
    let restored = inner.pending.take_commands(true);
    assert_eq!(
        restored,
        vec![NativeCommand::MapViews {
            clock_id: Some("page-A".into()),
            views: Vec::new(),
        }],
        "restoring an empty active dashboard must also stamp the active origin"
    );
}
