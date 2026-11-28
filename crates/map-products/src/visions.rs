use crate::ProductHour;
use std::{
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::{perspective::{self, Choice, Vision}, visibility::FogContent};

#[derive(Default)]
pub struct Visions {
    fold: Mutex<Option<viewer_foundation::perspective::incremental::Fold>>,
}
impl Visions {
    pub fn take(&self) -> Option<viewer_foundation::perspective::incremental::Fold> {
        self.fold.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    #[cfg(test)]
    pub fn stats(&self) -> Option<viewer_foundation::perspective::incremental::WorkStats> {
        self.fold.lock().unwrap().as_ref().map(|fold| fold.last)
    }
    pub fn get(&self, hour: &ProductHour<'_>, choice: &Choice) -> io::Result<Arc<Vision>> {
        for table in perspective::VISION_TABLES {
            let _ = &*hour.frame.tables[table.index()];
        }
        viewer_foundation::perspective::incremental::Fold::advance(
            &mut self
                .fold
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            hour.frame,
            choice,
            FogContent::loaded(&hour.content.fog),
        )
    }
}
