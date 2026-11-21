use crate::UnsavedWork;
use tauri::{AppHandle, Manager};

pub struct Host(pub AppHandle);

impl Host {
    fn unsaved(&self) -> Option<String> {
        self.0.state::<UnsavedWork>().update_refused()
    }
}

impl app_update::Host for Host {
    fn busy(&self) -> Option<String> {
        self.unsaved()
    }

    fn install(&self, launch: app_update::Launch) -> Result<(), String> {
        if let Some(reason) = self.unsaved() {
            return Err(reason);
        }
        launch()?;
        self.0.exit(0);
        Ok(())
    }

    fn close_for_update(&self) -> Result<(), String> {
        self.unsaved().map_or(Ok(()), Err)
    }

    fn closed_for_update(&self) {
        self.0.exit(0);
    }
}
