use crate::worker::{Leave, Recorder};
use std::sync::Arc;

pub struct Host(pub Arc<Recorder>);

impl app_update::Host for Host {
    fn install_blocked(&self) -> Option<String> {
        self.0.update_blocked().map(str::to_owned)
    }

    fn install(&self, launch: app_update::Launch) -> Result<(), String> {
        self.0.leave(Leave::Install(launch))
    }

    fn close_for_update(&self) -> Result<(), String> {
        self.0.leave(Leave::Exit)
    }

    fn write_preference(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<Option<String>> {
        let receipt = self.0.admit(replay_settings::exit::What::Change).map_err(std::io::Error::other)?;
        Ok(receipt.replace(path, |out| std::io::Write::write_all(out, bytes))?.warning)
    }
}

#[cfg(test)]
mod tests {
    use super::Host;
    use app_update::Host as _;
    use std::sync::Arc;

    #[test]
    fn an_update_channel_change_asked_for_once_the_recorder_quits_is_refused_and_writes_nothing() {
        let folder = std::env::temp_dir().join(format!("battleplan-recorder-channel-{}", std::process::id()));
        let path = folder.join("update-channel");
        let recorder = Arc::new(crate::worker::Recorder::default());
        let host = Host(Arc::clone(&recorder));
        recorder.commit_exit();
        assert!(host.write_preference(&path, b"nightly").is_err(), "the channel writer is admitted by the recorder’s own exit");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
