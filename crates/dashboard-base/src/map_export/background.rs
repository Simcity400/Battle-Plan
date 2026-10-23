use super::*;
use std::sync::{Mutex, Weak};

struct State {
    source: Weak<Replay>,
    epoch: Arc<()>,
    reader: Option<Result<Reader, String>>,
}

pub struct Background {
    state: Arc<Mutex<State>>,
}
impl Default for Background {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                source: Weak::new(),
                epoch: Arc::new(()),
                reader: None,
            })),
        }
    }
}
impl Background {
    pub fn start(
        &self,
        replay: Arc<Replay>,
        bind: impl FnOnce(&Replay, &dyn Fn() -> io::Result<()>) -> io::Result<Reader> + Send + 'static,
    ) -> Option<std::thread::JoinHandle<()>> {
        let epoch = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.source.ptr_eq(&Arc::downgrade(&replay)) {
                return None;
            }
            state.source = Arc::downgrade(&replay);
            state.epoch = Arc::new(());
            state.reader = None;
            Arc::clone(&state.epoch)
        };
        let state = Arc::downgrade(&self.state);
        let failure = state.clone();
        let failed_epoch = Arc::clone(&epoch);
        let spawn = std::thread::Builder::new()
            .name("map-export-bind".into())
            .spawn(move || {
                let check = || {
                    let Some(state) = state.upgrade() else {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "map export content retired",
                        ));
                    };
                    let state = state.lock().unwrap_or_else(|error| error.into_inner());
                    if !Arc::ptr_eq(&state.epoch, &epoch) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "map export source epoch retired",
                        ));
                    }
                    Ok(())
                };
                if check().is_err() {
                    return;
                }
                let result = bind(&replay, &check).map_err(|error| error.to_string());
                if let Some(state) = state.upgrade() {
                    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                    if Arc::ptr_eq(&state.epoch, &epoch)
                        && state.source.ptr_eq(&Arc::downgrade(&replay))
                    {
                        state.reader = Some(result);
                    }
                }
            });
        match spawn {
            Ok(handle) => Some(handle),
            Err(error) => {
                if let Some(state) = failure.upgrade() {
                    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                    if Arc::ptr_eq(&state.epoch, &failed_epoch) {
                        state.reader = Some(Err(error.to_string()));
                    }
                }
                None
            }
        }
    }
    pub fn reader(&self, replay: &Arc<Replay>) -> Result<Reader, String> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.source.ptr_eq(&Arc::downgrade(replay)) {
            return Err("map export: source epoch differs".into());
        }
        state
            .reader
            .clone()
            .unwrap_or_else(|| Err("map export: binding pending".into()))
    }
}
