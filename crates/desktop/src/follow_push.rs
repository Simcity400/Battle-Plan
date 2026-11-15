//! Followed answers and text panels pushed by the map's playheads in this process
//! ([`viewer::host::install_follow`]), each payload sent to the webview as a raw body on the
//! channel it registered (`dashboard_follow_channel`): an `ArrayBuffer` there, no JSON and no
//! base64, its first bytes naming its kind.

use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use tauri::ipc::{Channel, InvokeResponseBody};

#[derive(Default)]
pub struct FollowPush {
    channel: Mutex<Option<Channel<InvokeResponseBody>>>,
    sent: AtomicU64,
    dropped: AtomicU64,
}

impl FollowPush {
    /// Sends `payload` to the webview; dropped (and counted) while no channel is registered.
    pub fn send(&self, payload: Vec<u8>) {
        let channel = self.channel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        match channel.map(|channel| channel.send(InvokeResponseBody::Raw(payload))) {
            Some(Ok(())) => self.sent.fetch_add(1, Ordering::Relaxed),
            _ => self.dropped.fetch_add(1, Ordering::Relaxed),
        };
    }
}

#[tauri::command]
pub fn dashboard_follow_channel(push: tauri::State<'_, FollowPush>, channel: Channel<InvokeResponseBody>) {
    *push.channel.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(channel);
}
