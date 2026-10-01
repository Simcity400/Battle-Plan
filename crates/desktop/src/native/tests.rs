use super::line_reader::read_lines;
use super::*;

#[test]
fn full_data_queues_refuse_transiently_and_accept_again_after_draining() {
    let request = |id: usize| DataCommand::Open {
        id: id.to_string(),
        path: "fixture.battleplan".into(),
        revision: "1".into(),
    };
    for (count, length) in [(128, 1), (64, 32 * 1024)] {
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
    let mut input = vec![b'x'; MAX_MESSAGE_BYTES * 3];
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
