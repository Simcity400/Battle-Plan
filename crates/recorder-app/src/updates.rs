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

    fn write_preference(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        replay::writes::WRITES.begin()?.replace(path, |out| std::io::Write::write_all(out, bytes))
    }
}
