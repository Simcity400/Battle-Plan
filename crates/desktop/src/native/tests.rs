use super::line_reader::read_lines;
use super::pending::{MAX_QUEUED_ACTIONS, MAX_QUEUED_DATA, MAX_QUEUED_DATA_BYTES};
use super::*;

#[test]
fn full_data_queues_refuse_transiently_and_accept_again_after_draining() {
    let request = |id: usize| DataCommand::Open {
        id: id.to_string(),
        path: "fixture.battleplan".into(),
        revision: "1".into(),
    };
    for (count, length) in [(128, 1), (64, MAX_QUEUED_DATA_BYTES / 64)] {
        let mut pending = Pending::default();
        for id in 0..count {
            pending.enqueue_data(request(id), length).unwrap();
        }
        let refusal = pending.enqueue_data(request(count), length).unwrap_err();
        assert_eq!(refusal, dashboard::transport::NATIVE_QUEUE_FULL);
        assert!(dashboard::transport::TRANSIENT_ERRORS.contains(&refusal.as_str()));
        assert_eq!(pending.data.len(), count);
        pending.take_commands(false);
        pending.enqueue_data(request(count), length).unwrap();
    }
}

#[test]
fn country_requests_fit_the_native_pipe_and_queue_until_the_shared_byte_budget() {
    let mut request = DataCommand::Product {
        id: "request-1".into(),
        request: dashboard::transport::ProductRequest {
            source_id: "source-1".into(),
            revision: "1".into(),
            product: "agency-comparison".into(),
            params: serde_json::json!({
                "branchId": "1",
                "top": null,
                "tags": (0..5000)
                    .map(|index| format!("synthetic-country-{index:04}"))
                    .collect::<Vec<_>>()
            }),
        },
    };
    let host = NativeHost::default();
    let length = serde_json::to_vec(&NativeCommand::Data {
        request: request.clone(),
    })
    .unwrap()
    .len()
        + 1;
    assert!(length > 64 * 1024);
    // A missing process is reached only after the request's byte admission.
    assert_eq!(
        host.enqueue_data(request.clone()).unwrap_err(),
        "The native data connection closed"
    );
    let mut pending = Pending::default();
    pending.enqueue_data(request.clone(), length).unwrap();
    assert_eq!(pending.data.len(), 1);
    if let DataCommand::Product { request, .. } = &mut request {
        request.params["tags"] = serde_json::json!([""]);
    }
    let overhead = serde_json::to_vec(&NativeCommand::Data {
        request: request.clone(),
    })
    .unwrap()
    .len()
        + 1;
    if let DataCommand::Product { request, .. } = &mut request {
        request.params["tags"] = serde_json::json!(["x".repeat(MAX_MESSAGE_BYTES - overhead)]);
    }
    assert_eq!(
        host.enqueue_data(request.clone()).unwrap_err(),
        "The native data connection closed"
    );
    Pending::default()
        .enqueue_data(request.clone(), MAX_MESSAGE_BYTES)
        .unwrap();
    if let DataCommand::Product { request, .. } = &mut request {
        request.params["tags"] = serde_json::json!(["x".repeat(MAX_MESSAGE_BYTES - overhead + 1)]);
    }
    assert_eq!(
        host.enqueue_data(request.clone()).unwrap_err(),
        "Dashboard request exceeds the native control message budget"
    );
    let refusal = Pending::default()
        .enqueue_data(request, MAX_MESSAGE_BYTES + 1)
        .unwrap_err();
    assert_eq!(refusal, dashboard::transport::NATIVE_QUEUE_FULL);
    assert!(dashboard::transport::transient_error(&refusal));
}

#[test]
fn cancellation_removes_its_queued_request_before_admission_at_capacity() {
    let mut pending = Pending::default();
    for id in 0..128 {
        pending
            .enqueue_data(
                DataCommand::Open {
                    id: id.to_string(),
                    path: "fixture.battleplan".into(),
                    revision: "1".into(),
                },
                1,
            )
            .unwrap();
    }
    pending
        .enqueue_data(DataCommand::Cancel { id: "0".into() }, 1)
        .unwrap();
    assert_eq!(pending.data.len(), 128);
    assert!(matches!(
        pending.data.back(),
        Some((DataCommand::Cancel { .. }, _))
    ));
    assert!(
        !pending
            .data
            .iter()
            .any(|(command, _)| matches!(command, DataCommand::Open { id, .. } if id == "0"))
    );
}

#[test]
fn resized_commands_coalesce_and_close_takes_priority() {
    let mut pending = Pending {
        visibility: Some(true),
        open: Some(("x".into(), "v1".into())),
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
fn oversized_lines_do_not_hide_the_next_message() {
    let mut input = vec![b'x'; MAX_MESSAGE_BYTES + 2];
    input.extend_from_slice(b"\nvalid\n");
    let mut lines = Vec::new();
    read_lines(input.as_slice(), |line| lines.push(line.to_owned()));
    assert_eq!(lines, ["valid"]);
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
        Some(("new.battleplan".into(), "new-version".into()))
    );
    assert_eq!(inner.pending.seek, Some(requested));
    inner.requested_path = None;
    inner.requested_revision = None;
    inner.requested_seek = None;
    inner.restore_commands();
    assert_eq!(
        inner.pending.open,
        Some(("old.battleplan".into(), "old-version".into()))
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
        open: Some(("next.battleplan".into(), "v2".into())),
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

use dashboard::transport::{MAP_CONTROLS_BUSY, NATIVE_QUEUE_FULL, transient_error};

fn open(id: String) -> DataCommand {
    DataCommand::Open {
        id,
        path: "a.battleplan".into(),
        revision: "v1".into(),
    }
}

#[test]
fn a_full_data_queue_refuses_with_a_retryable_error_and_admits_again_once_sent() {
    let mut pending = Pending::default();
    for index in 0..MAX_QUEUED_DATA {
        pending
            .enqueue_data(open(format!("fill-{index}")), 100)
            .unwrap();
    }
    // Full by count: the refusal says only that there was no room.
    let refusal = pending.enqueue_data(open("late".into()), 100).unwrap_err();
    assert_eq!(refusal, NATIVE_QUEUE_FULL);
    assert!(transient_error(&refusal));
    // Once the writer sends some, the same request is taken.
    assert_eq!(pending.take_commands(false).len(), 4);
    pending.enqueue_data(open("late".into()), 100).unwrap();
    // Full by bytes: the same refusal.
    let mut pending = Pending::default();
    pending
        .enqueue_data(open("large".into()), MAX_QUEUED_DATA_BYTES - 10)
        .unwrap();
    let refusal = pending.enqueue_data(open("next".into()), 11).unwrap_err();
    assert!(transient_error(&refusal), "{refusal}");
    // A cancel frees its own queued request's place.
    pending
        .enqueue_data(DataCommand::Cancel { id: "large".into() }, 11)
        .unwrap();
    assert_eq!(pending.data.len(), 1);
}

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
        background_preparation: None,
    };
    host.set_folders(&config);
    let inner = host.shared.0.lock().unwrap();
    assert_eq!(inner.folders.as_ref(), Some(&config));
    // No viewer runs, so nothing is queued for one.
    assert!(inner.pending.actions.is_empty());
}

#[test]
fn display_choices_sent_while_no_viewer_runs_are_kept_for_its_launch() {
    use dashboard::protocol::MapAction;
    let host = NativeHost::popout(1, Arc::new(|| ()));
    host.set_display(MapAction::Vsync { enabled: true });
    host.set_display(MapAction::Vsync { enabled: false });
    host.set_display(MapAction::UiScale { scale: Some(1.5) });
    let inner = host.shared.0.lock().unwrap();
    // Nothing runs yet, so nothing is queued; the latest of each kind is kept instead.
    assert!(inner.pending.actions.is_empty());
    assert_eq!(
        format!("{:?}", inner.display),
        format!(
            "{:?}",
            [
                MapAction::Vsync { enabled: false },
                MapAction::UiScale { scale: Some(1.5) }
            ]
        )
    );
    // A pop-out host shows the campaign map and reports through its owner.
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
            .0
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
    let inner = host.shared.0.lock().unwrap();
    // No viewer runs and none is started; the next one started takes the bounds at launch.
    assert!(inner.process.is_none());
    assert!(!inner.status.running);
    assert_eq!(inner.bounds, Some(bounds));
    assert_eq!(inner.pending.bounds, None);
}

#[test]
fn a_display_choice_sent_while_a_viewer_starts_replaces_its_launch_read() {
    use dashboard::protocol::MapAction;
    // Read at launch, then VSync changed before the child was installed.
    let read = vec![
        MapAction::Vsync { enabled: true },
        MapAction::UiScale { scale: None },
    ];
    let sent = [MapAction::Vsync { enabled: false }];
    let debug = |actions: &[MapAction]| format!("{actions:?}");
    assert_eq!(
        debug(&launch_display(read.clone(), &sent)),
        debug(&[
            MapAction::UiScale { scale: None },
            MapAction::Vsync { enabled: false }
        ])
    );
    // Nothing sent leaves the launch read as it is.
    assert_eq!(debug(&launch_display(read.clone(), &[])), debug(&read));
}

#[test]
fn a_host_names_the_recording_it_shows_requests_and_seeks() {
    let host = NativeHost::popout(1, Arc::new(|| ()));
    assert!(host.recordings().is_empty());
    {
        let mut inner = host.shared.0.lock().unwrap();
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
fn a_waiting_controller_message_wakes_the_writer_even_while_hidden() {
    use dashboard::protocol::GamepadMessage;
    let mut pending = Pending::default();
    assert!(!pending.can_send(true));
    pending.gamepad = Some(GamepadMessage::Release);
    // A release must reach a hidden viewer too.
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
    host.shared.0.lock().unwrap().status.ready = true;
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
    let inner = host.shared.0.lock().unwrap();
    let queued: Vec<_> = inner
        .pending
        .actions
        .iter()
        .map(|action| match action {
            MapAction::CameraMode { mode, overlays } => Some((*mode, *overlays)),
            _ => None,
        })
        .collect();
    // The latest mode keeps the latest overlay choice with it.
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

/// Stepping through many planes queues only the last choice, beside the camera mode, so
/// the action queue never fills.
#[test]
fn only_the_latest_aircraft_choice_waits_for_the_viewer() {
    use dashboard::protocol::{CameraMode, MapAction};
    let host = NativeHost::default();
    host.shared.0.lock().unwrap().status.ready = true;
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
    let inner = host.shared.0.lock().unwrap();
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
