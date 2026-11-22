use super::Inner;
use dashboard_core::protocol::{DashboardMapView, NativeCommand, ViewBounds, ViewerState};
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SeekTarget {
    pub(super) path: String,
    pub(super) revision: String,
    pub(super) branch: String,
    pub(super) hour: i32,
    pub(super) playback: Option<dashboard_core::protocol::MapPlayback>,
}

impl SeekTarget {
    fn command(self) -> NativeCommand {
        NativeCommand::Seek {
            path: self.path,
            revision: self.revision,
            branch: self.branch,
            hour: self.hour,
            playback: self.playback,
        }
    }

    pub(super) fn matches(&self, state: &ViewerState) -> bool {
        state.path.as_ref() == Some(&self.path)
            && state.revision.as_ref() == Some(&self.revision)
            && state.branch.as_ref() == Some(&self.branch)
            && state.hour == Some(self.hour)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DashboardSnapshot {
    pub(super) clock_id: Option<String>,
    pub(super) views: Vec<DashboardMapView>,
}

#[derive(Default)]
pub(super) struct Pending {
    pub(super) actions: VecDeque<dashboard_core::protocol::MapAction>,
    pub(super) campaign_map: bool,
    pub(super) bounds: Option<ViewBounds>,
    pub(super) visibility: Option<bool>,
    pub(super) open: Option<(String, Option<String>)>,
    pub(super) seek: Option<SeekTarget>,
    pub(super) maps: Option<DashboardSnapshot>,
    pub(super) gamepad: Option<dashboard_core::protocol::GamepadMessage>,
    pub(super) close: bool,
}

pub(super) const MAX_QUEUED_ACTIONS: usize = 16;

impl Pending {
    pub(super) fn action_room(&self) -> Result<(), String> {
        if self.actions.len() >= MAX_QUEUED_ACTIONS {
            return Err(format!(
                "{}; try again",
                dashboard_core::transport::MAP_CONTROLS_BUSY
            ));
        }
        Ok(())
    }

    pub(super) fn can_send(&self, visible: bool) -> bool {
        self.close
            || !self.actions.is_empty()
            || self.campaign_map
            || self.maps.is_some()
            || self.bounds.is_some()
            || self.visibility.is_some()
            || self.gamepad.is_some()
            || (visible && (self.open.is_some() || self.seek.is_some()))
    }

    pub(super) fn take_commands(&mut self, visible: bool) -> Vec<NativeCommand> {
        if self.close {
            *self = Self::default();
            return vec![NativeCommand::Close];
        }
        let mut commands = Vec::with_capacity(4);
        commands.extend(
            self.actions
                .drain(..)
                .map(|action| NativeCommand::MapAction { action }),
        );
        if std::mem::take(&mut self.campaign_map) {
            commands.push(NativeCommand::CampaignMap);
        }
        if let Some(bounds) = self.bounds.take() {
            commands.push(bounds.command());
        }
        if visible && let Some((path, revision)) = self.open.take() {
            commands.push(NativeCommand::Open { path, revision });
        }
        if visible && let Some(target) = self.seek.take() {
            commands.push(target.command());
        }
        if let Some(visible) = self.visibility.take() {
            commands.push(NativeCommand::Visibility { visible });
        }
        if let Some(snapshot) = self.maps.take() {
            commands.push(NativeCommand::MapViews {
                clock_id: snapshot.clock_id,
                views: snapshot.views,
            });
        }
        if let Some(message) = self.gamepad.take() {
            commands.push(NativeCommand::Gamepad { message });
        }
        commands
    }
}

impl Inner {
    pub(super) fn forget_recording(&mut self, path: &str) {
        if self
            .pending
            .open
            .as_ref()
            .is_some_and(|(pending, _)| pending == path)
        {
            self.pending.open = None;
        }
        if self
            .pending
            .seek
            .as_ref()
            .is_some_and(|pending| pending.path == path)
        {
            self.pending.seek = None;
        }
        if self.requested_path.as_deref() == Some(path) {
            self.requested_path = None;
            self.requested_revision = None;
        }
        if self
            .requested_seek
            .as_ref()
            .is_some_and(|pending| pending.path == path)
        {
            self.requested_seek = None;
        }
        if self.status.viewer.path.as_deref() == Some(path) {
            self.status.viewer = ViewerState::default();
        }
    }

    pub(super) fn restore_commands(&mut self) {
        if self.active_dashboard {
            self.pending.maps = Some(self.dashboard_snapshot());
            return;
        }
        let target = self
            .requested_path
            .clone()
            .zip(self.requested_revision.clone())
            .or_else(|| {
                self.requested_seek
                    .as_ref()
                    .map(|target| (target.path.clone(), target.revision.clone()))
            })
            .or_else(|| {
                self.status
                    .viewer
                    .path
                    .clone()
                    .zip(self.status.viewer.revision.clone())
            });
        if let Some((path, revision)) = target {
            let last_position = (self.status.viewer.path.as_ref() == Some(&path)
                && self.status.viewer.revision.as_ref() == Some(&revision))
            .then(|| {
                self.status
                    .viewer
                    .branch
                    .clone()
                    .zip(self.status.viewer.hour)
                    .map(|(branch, hour)| SeekTarget {
                        path: path.clone(),
                        revision: revision.clone(),
                        branch,
                        hour,
                        playback: None,
                    })
            })
            .flatten();
            self.pending.open = Some((path, Some(revision)));
            self.pending.seek = self.requested_seek.clone().or(last_position);
        }
    }
}
