use crate::leaving::Leaving;
use tauri::{AppHandle, Manager};

pub struct Host(pub AppHandle);

impl Host {
    fn leaving(&self) -> &Leaving {
        self.0.state::<Leaving>().inner()
    }
}

impl app_update::Host for Host {
    fn busy(&self) -> Option<String> {
        self.leaving().busy()
    }

    fn install(&self, launch: app_update::Launch) -> Result<(), String> {
        self.leaving().install(launch)
    }

    fn close_for_update(&self) -> Result<(), String> {
        self.leaving().close_for_update()
    }

    fn write_preference(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<Option<String>> {
        Ok(replay_settings::exit::EXIT.admit(replay_settings::exit::What::Change)?.replace(path, |out| std::io::Write::write_all(out, bytes))?.warning)
    }
}
