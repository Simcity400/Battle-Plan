//! Battle Plan's part in updates: it waits for the recorder, then closes for the installer.
use tauri::AppHandle;

pub struct Host(pub AppHandle);

impl app_update::Host for Host {
    /// The recorder must not be replaced while it records; this app has nothing to wait for.
    fn install_blocked(&self) -> Option<String> {
        app_update::install_blocked(app_update::Role::Recorder)
    }

    fn install(&self, launch: app_update::Launch) -> Result<(), String> {
        launch()?;
        self.0.exit(0);
        Ok(())
    }

    fn close_for_update(&self) -> Result<(), String> {
        Ok(())
    }

    fn closed_for_update(&self) {
        self.0.exit(0);
    }
}
