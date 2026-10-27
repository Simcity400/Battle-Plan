use crate::ProductHour;
use std::{
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::map::{
    perspective::{self, Choice, Vision},
    visibility::FogContent,
};

pub struct Visions {
    fold: Mutex<Option<viewer_foundation::map::perspective::incremental::Fold>>,
    budget: viewer_foundation::map::perspective::incremental::Memory,
}
impl Visions {
    pub fn with_budget(budget: viewer_foundation::map::perspective::incremental::Memory) -> Self {
        Self {
            fold: Mutex::new(None),
            budget,
        }
    }
    #[cfg(test)]
    pub fn stats(&self) -> Option<viewer_foundation::map::perspective::incremental::WorkStats> {
        self.fold.lock().unwrap().as_ref().map(|fold| fold.last)
    }
    pub fn budget(&self) -> &map_display_data::credit::MemoryBudget {
        self.budget.budget()
    }
    pub fn get(&self, hour: &ProductHour<'_>, choice: &Choice) -> io::Result<Arc<Vision>> {
        for table in perspective::VISION_TABLES {
            let _ = &*hour.frame.tables[table.index()];
        }
        viewer_foundation::map::perspective::incremental::Fold::advance(
            &mut self
                .fold
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            hour.frame,
            choice,
            FogContent::loaded(&hour.content.fog),
            self.budget.budget(),
        )
    }
}
