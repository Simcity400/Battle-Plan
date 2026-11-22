use crate::PageLeaving;
use tauri::{AppHandle, Manager};

pub struct Host(pub AppHandle);

impl Host {
    fn leaving(&self) -> &PageLeaving {
        self.0.state::<PageLeaving>().inner()
    }
}

impl app_update::Host for Host {
    fn busy(&self) -> Option<String> {
        self.leaving().busy()
    }

    fn install(&self, launch: app_update::Launch, confirmed: bool) -> Result<(), String> {
        self.leaving().install(confirmed, launch)?;
        self.0.exit(0);
        Ok(())
    }

    fn close_for_update(&self) -> Result<(), String> {
        self.leaving().close_for_update()
    }

    fn closed_for_update(&self) {
        self.0.exit(0);
    }
}
