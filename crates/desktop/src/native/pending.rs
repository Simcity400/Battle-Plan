use super::Inner;
use dashboard::protocol::{DashboardMapView, NativeCommand, ViewBounds, ViewerState};
use dashboard::transport::DataCommand;
#[cfg(test)]
use dashboard::transport::NATIVE_QUEUE_FULL;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SeekTarget {
    pub(super) path: String,
    pub(super) revision: String,
    pub(super) branch: String,
    pub(super) hour: i32,
    pub(super) playback: Option<dashboard::protocol::MapPlayback>,
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

#[derive(Default)]
pub(super) struct Pending {
    pub(super) actions: VecDeque<dashboard::protocol::MapAction>,
    /// Select the Campaign map over the dashboard's maps.
    pub(super) campaign_map: bool,
    pub(super) bounds: Option<ViewBounds>,
    pub(super) visibility: Option<bool>,
    pub(super) open: Option<(String, String)>,
    pub(super) seek: Option<SeekTarget>,
    pub(super) maps: Option<Vec<DashboardMapView>>,
    pub(super) data: VecDeque<(DataCommand, usize)>,
    /// The controller message waiting for the viewer, merged with any sent after it
    /// ([`GamepadMessage::merge`](dashboard::protocol::GamepadMessage::merge)).
    pub(super) gamepad: Option<dashboard::protocol::GamepadMessage>,
    pub(super) close: bool,
}

/// Data requests queued for the native pipe at most, by count and by bytes.
#[cfg(test)]
pub(super) const MAX_QUEUED_DATA: usize = 128;
#[cfg(test)]
pub(super) const MAX_QUEUED_DATA_BYTES: usize = dashboard::protocol::MAX_MESSAGE_BYTES;
/// Map actions queued at most.
pub(super) const MAX_QUEUED_ACTIONS: usize = 16;

impl Pending {
    /// Queues a data request of `length` serialized bytes. A cancel first removes its
    /// request if that has not left the queue (a copy the writer already took precedes
    /// the cancel on its pipe). A full queue refuses with [`NATIVE_QUEUE_FULL`], which
    /// says only that there was no room, so the frontend tries the request again.
    ///
    /// [`NATIVE_QUEUE_FULL`]: dashboard::transport::NATIVE_QUEUE_FULL
    #[cfg(test)]
    pub(super) fn enqueue_data(
        &mut self,
        request: DataCommand,
        length: usize,
    ) -> Result<(), String> {
        if let DataCommand::Cancel { id } = &request {
            self.data
                .retain(|(queued, _)| queued.request_id() != Some(id.as_str()));
        }
        let bytes: usize = self.data.iter().map(|(_, bytes)| *bytes).sum();
        if self.data.len() >= MAX_QUEUED_DATA
            || bytes.saturating_add(length) > MAX_QUEUED_DATA_BYTES
        {
            return Err(NATIVE_QUEUE_FULL.into());
        }
        self.data.push_back((request, length));
        Ok(())
    }

    /// Whether another map action fits. A full queue refuses with
    /// [`MAP_CONTROLS_BUSY`], which the frontend may try again.
    ///
    /// [`MAP_CONTROLS_BUSY`]: dashboard::transport::MAP_CONTROLS_BUSY
    pub(super) fn action_room(&self) -> Result<(), String> {
        if self.actions.len() >= MAX_QUEUED_ACTIONS {
            return Err(format!(
                "{}; try again",
                dashboard::transport::MAP_CONTROLS_BUSY
            ));
        }
        Ok(())
    }

    pub(super) fn can_send(&self, visible: bool) -> bool {
        self.close
            || !self.actions.is_empty()
            || self.campaign_map
            || self.maps.is_some()
            || !self.data.is_empty()
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
        // Analytics can open another recording without loading its map assets.
        // Keep only the latest document request until Map becomes visible.
        if visible && let Some((path, revision)) = self.open.take() {
            commands.push(NativeCommand::Open { path, revision });
        }
        if visible && let Some(target) = self.seek.take() {
            commands.push(target.command());
        }
        if let Some(visible) = self.visibility.take() {
            commands.push(NativeCommand::Visibility { visible });
        }
        if let Some(views) = self.maps.take() {
            commands.push(NativeCommand::MapViews { views });
        }
        if let Some(message) = self.gamepad.take() {
            commands.push(NativeCommand::Gamepad { message });
        }
        // Drain a small amount per pass so another seek/layout can interrupt a
        // batch of history demands. The queue itself has a byte/count ceiling.
        for _ in 0..4 {
            let Some((request, _)) = self.data.pop_front() else {
                break;
            };
            commands.push(NativeCommand::Data { request });
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

    /// Pending pipe slots may already have been sent when a process exits.
    /// Retain unacknowledged requests, otherwise reopen its last displayed hour.
    pub(super) fn restore_commands(&mut self) {
        if self.active_dashboard {
            let views = self.dashboard_views.clone().unwrap_or_default();
            self.pending.maps = Some(views);
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
            self.pending.open = Some((path, revision));
            self.pending.seek = self.requested_seek.clone().or(last_position);
        }
    }
}
