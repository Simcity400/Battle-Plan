
use std::{
    any::Any,
    cell::RefCell,
    path::Path,
    sync::{Arc, OnceLock},
};

thread_local! {
    static ROOM: RefCell<Option<Room>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct Room {
    folder: Arc<Path>,
    kept: Arc<OnceLock<Arc<dyn Any + Send + Sync>>>,
}

impl Room {
    pub fn new(folder: &Path) -> Self {
        Self {
            folder: Arc::from(folder),
            kept: Arc::default(),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn lane(&self) -> Self {
        Self::new(&self.folder)
    }

    pub fn kept<T: Any + Send + Sync>(&self, make: impl FnOnce(&Path) -> T) -> Option<Arc<T>> {
        let held = self.kept.get_or_init(|| Arc::new(make(&self.folder)));
        Arc::clone(held).downcast::<T>().ok()
    }
}

pub fn room() -> Option<Room> {
    ROOM.with(|held| held.borrow().clone())
}

pub fn enter(room: Option<Room>) -> Entered {
    Entered {
        previous: ROOM.with(|held| held.replace(room)),
    }
}

#[must_use]
pub struct Entered {
    previous: Option<Room>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        let previous = self.previous.take();
        ROOM.with(|held| *held.borrow_mut() = previous);
    }
}
